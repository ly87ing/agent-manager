#!/usr/bin/env bash

set -euo pipefail

# Color definitions
HDR_BLUE='\033[1;34m'   
AGT_BLACK='\033[1;30m'  
VAL_PURPLE='\033[0;35m' 
BORDER_GRAY='\033[90m'  
SUCCESS_GREEN='\033[0;32m'
PENDING_GOLD='\033[0;33m'
ERROR_RED='\033[0;31m'
NC='\033[0m' 
BOLD='\033[1m'
WHITE='\033[1;37m'

# Resolve the absolute path of the script's root directory
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
USER_NAME=$(whoami)
RUN_ID="${AGENT_MANAGER_RUN_ID:-$(date +%Y%m%d%H%M%S)_$$}"
LOG_PREFIX="${AGENT_MANAGER_LOG_PREFIX:-/tmp/agent_manager_sync_${USER_NAME}}"
TEMP_LOG="${LOG_PREFIX}_${RUN_ID}.log"
LATEST_LOG_POINTER="${AGENT_MANAGER_LATEST_LOG_POINTER:-${LOG_PREFIX}.latest}"
LEGACY_LOG_PATH="${AGENT_MANAGER_LEGACY_LOG_PATH:-${LOG_PREFIX}.log}"
LOCK_FILE="${AGENT_MANAGER_LOCK_FILE:-/tmp/agent_manager_${USER_NAME}.lock}"
LOCK_DIR="${LOCK_FILE}.d"
LOCK_PID_FILE="${LOCK_DIR}/pid"
# Baseline policy (provided by the repo)
BASE_CONFIG="${AGENT_MANAGER_BASE_CONFIG:-$ROOT_DIR/config/manager.json}"
# Synced shared layer (managed by the profile sync engine, not tracked by Git)
SYNCED_CONFIG="${AGENT_MANAGER_SYNCED_CONFIG:-$ROOT_DIR/config/manager.synced.json}"
# Local overrides (not tracked by Git)
LOCAL_CONFIG="${AGENT_MANAGER_LOCAL_CONFIG:-$ROOT_DIR/config/manager.local.json}"
LOCK_ACQUIRED="false"
DRY_RUN_FLAG=""
OFFLINE_FLAG=""
FORCE_REFRESH_FLAG=""
WAIT_AGENT_LIFECYCLE_FLAG=""
TASK_FAILURES=0
DOCTOR_STATUS="healthy"
DOCTOR_WARNING_OPTIONAL_TOOLS=0
DOCTOR_WARNING_UNMATERIALIZED=0
TEMP_ARTIFACTS=()
ORIGINAL_ARGS=("$@")
CMD_ARGS=()
cmd=""

# If the synced shared layer or the local override file exists, read them and deep-merge
if [ -f "$SYNCED_CONFIG" ] || [ -f "$LOCAL_CONFIG" ]; then
    CONFIG_FILE="$(mktemp)"
    if ! python3 -c "
import json, os, sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping
from src.json_config import load_json_object_file
base = load_json_object_file('$BASE_CONFIG', strict=True, missing_ok=False)
def deep_update(dst, src):
    for k, v in src.items():
        if isinstance(v, dict) and isinstance(dst.get(k), dict):
            deep_update(dst[k], v)
        else:
            dst[k] = v
if os.path.isfile('$SYNCED_CONFIG'):
    try:
        synced = load_json_object_file('$SYNCED_CONFIG', strict=True, missing_ok=False)
    except ValueError as e:
        print(f'共享层配置无效: {e}', file=sys.stderr)
        sys.exit(2)
    deep_update(base, synced)
if os.path.isfile('$LOCAL_CONFIG'):
    try:
        local = load_json_object_file('$LOCAL_CONFIG', strict=True, missing_ok=False)
    except ValueError as e:
        print(f'本地覆盖配置无效: {e}', file=sys.stderr)
        sys.exit(2)
    deep_update(base, local)
if 'agents' in base:
    base['agents'] = canonicalize_agent_mapping(base.get('agents', {}))
with open('$CONFIG_FILE', 'w', encoding='utf-8') as f:
    json.dump(base, f, indent=2, ensure_ascii=False)
"; then
        rm -f "$CONFIG_FILE"
        exit 1
    fi
else
    # Use the baseline directly when there is no synced or local override
    CONFIG_FILE="$BASE_CONFIG"
fi

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run)
            DRY_RUN_FLAG="--dry-run"
            ;;
        --offline)
            OFFLINE_FLAG="--offline"
            ;;
        -f|--force-refresh)
            FORCE_REFRESH_FLAG="--force-refresh"
            ;;
        --wait-agent-lifecycle)
            WAIT_AGENT_LIFECYCLE_FLAG="--wait-async-actions"
            ;;
        *)
            if [ -z "$cmd" ]; then
                cmd="$1"
            else
                CMD_ARGS+=("$1")
            fi
            ;;
    esac
    shift
done
if [ -z "$cmd" ]; then
    # 全局 flag 只修饰命令，自身不构成命令。这里过去默认跑 init(含安装缺失 Agent),
    # 与 `--offline` 曾经对应的 offline(=apply) 语义不符,一个裸 flag 就可能误触全量安装。
    if [ -n "$DRY_RUN_FLAG" ] || [ -n "$OFFLINE_FLAG" ] || [ -n "$FORCE_REFRESH_FLAG" ] || [ -n "$WAIT_AGENT_LIFECYCLE_FLAG" ]; then
        echo -e "${ERROR_RED}错误: 只提供了全局参数，没有指定命令。${NC}" >&2
        echo -e "  预演完整 init: ${BOLD}./manage.sh init --dry-run${NC}   离线分发: ${BOLD}./manage.sh --offline update${NC}" >&2
        echo -e "  命令清单: ${BOLD}./manage.sh help${NC}" >&2
        exit 1
    fi
    cmd="help"
fi

function read_lock_pid() {
    if [ -f "$LOCK_PID_FILE" ]; then
        cat "$LOCK_PID_FILE" 2>/dev/null || true
        return
    fi
    if [ -f "$LOCK_FILE" ]; then
        cat "$LOCK_FILE" 2>/dev/null || true
    fi
}

function remove_lock_artifacts() {
    rm -rf "$LOCK_DIR"
    rm -f "$LOCK_FILE"
}

function pid_is_alive() {
    local pid="${1:-}"
    [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

function acquire_lock() {
    local legacy_pid=""
    if [ -f "$LOCK_FILE" ] && [ ! -d "$LOCK_DIR" ]; then
        legacy_pid="$(cat "$LOCK_FILE" 2>/dev/null || true)"
        if [ -n "$legacy_pid" ] && [[ "$legacy_pid" != "$$" ]] && pid_is_alive "$legacy_pid"; then
            echo -e "${ERROR_RED}错误: 另一个管理任务 (PID: $legacy_pid) 正在运行。${NC}"
            echo -e "\n${PENDING_GOLD}提示: 确认无任务运行后，可手动删除锁文件 $LOCK_FILE 与锁目录 $LOCK_DIR。${NC}"
            exit 1
        fi
        rm -f "$LOCK_FILE"
    fi

    if mkdir "$LOCK_DIR" 2>/dev/null; then
        printf '%s\n' "$$" > "$LOCK_PID_FILE"
        printf '%s\n' "$$" > "$LOCK_FILE"
        LOCK_ACQUIRED="true"
        return 0
    fi

    local old_pid
    old_pid="$(read_lock_pid)"
    if [ -n "$old_pid" ] && [[ "$old_pid" != "$$" ]] && pid_is_alive "$old_pid"; then
        echo -e "${ERROR_RED}错误: 另一个管理任务 (PID: $old_pid) 正在运行。${NC}"
        echo -e "\n${PENDING_GOLD}提示: 确认无任务运行后，可手动删除锁文件 $LOCK_FILE 与锁目录 $LOCK_DIR。${NC}"
        exit 1
    fi

    remove_lock_artifacts
    if mkdir "$LOCK_DIR" 2>/dev/null; then
        printf '%s\n' "$$" > "$LOCK_PID_FILE"
        printf '%s\n' "$$" > "$LOCK_FILE"
        LOCK_ACQUIRED="true"
        return 0
    fi

    echo -e "${ERROR_RED}错误: 无法获取运行锁，请稍后重试。${NC}"
    exit 1
}

# Atomic lock check（只读命令豁免：help/doctor、引擎契约的只读族 *:status / *:plan / system:audit / runs / snapshots:list / policy:get）
if [[ "$cmd" != "help" && "$cmd" != "-h" && "$cmd" != "--help" && "$cmd" != "-help" && "$cmd" != "doctor" && "$cmd" != *:status && "$cmd" != *:plan && "$cmd" != "system:audit" && "$cmd" != "runs:list" && "$cmd" != "runs:get" && "$cmd" != "snapshots:list" && "$cmd" != "policy:get" ]]; then
    acquire_lock
fi

function cleanup_on_exit() {
    if [ "$LOCK_ACQUIRED" = "true" ] && [ "$(read_lock_pid)" = "$$" ]; then
        remove_lock_artifacts
    fi
    [ -n "${CONFIG_FILE:-}" ] && [[ "$CONFIG_FILE" != "$BASE_CONFIG" ]] && rm -f "$CONFIG_FILE"
    local artifact
    if [ "${#TEMP_ARTIFACTS[@]}" -gt 0 ]; then
        for artifact in "${TEMP_ARTIFACTS[@]}"; do
            rm -f "$artifact" 2>/dev/null || true
        done
    fi

    local running_jobs=$(jobs -p)
    if [ -n "$running_jobs" ]; then
        kill $running_jobs 2>/dev/null || true
        wait $running_jobs 2>/dev/null || true
    fi
}

function handle_signal_exit() {
    local exit_code="$1"
    trap - EXIT
    cleanup_on_exit
    exit "$exit_code"
}

trap cleanup_on_exit EXIT
trap 'handle_signal_exit 143' TERM
trap 'handle_signal_exit 130' INT

# Ensure execute permissions
chmod +x "$ROOT_DIR/agent-rule/scripts/"*.sh > /dev/null 2>&1 || true

# System probe
OS_TYPE="linux"
if [[ "$OSTYPE" == "darwin"* ]]; then OS_TYPE="macos";
elif [[ "$OSTYPE" == "msys" || "$OSTYPE" == "cygwin" || "$OSTYPE" == "win32" ]]; then OS_TYPE="windows"; fi

PYTHON_CMD="python3"
command -v python3 >/dev/null 2>&1 || PYTHON_CMD="python"

function status_line() {
    local status="$1" message="$2"
    case "$status" in
        "RUNNING") printf "\r\033[K  ${PENDING_GOLD}⌛ %-10s${NC} │ %s" "进行中" "$message" ;;
        "DONE")    printf "\r\033[K  ${SUCCESS_GREEN}✅ %-10s${NC} │ %s\n" "已完成" "$message" ;;
        "SKIP")    printf "\r\033[K  ${HDR_BLUE}⏭️  %-10s${NC} │ %s\n" "已跳过" "$message" ;;
        "ERROR")   printf "\r\033[K  ${ERROR_RED}❌ %-10s${NC} │ %s\n" "失败" "$message" ;;
    esac
}

function format_progress_tail() {
    local message="$1"
    local limit="${2:-55}"
    "$PYTHON_CMD" - "$message" "$limit" <<'PY'
import re
import sys

message = sys.argv[1]
limit = int(sys.argv[2])

message = re.sub(r'^[0-9-]+ [0-9:]+ \[(INFO|WARNING|ERROR)\] ', '', message)
message = re.sub(r'^INFO: ', '', message)
message = message.replace('\r', '').strip()

if not message:
    sys.exit(0)

print(message[:limit], end='')
PY
}

function timeout_run() {
    local seconds="$1"; shift
    if command -v timeout >/dev/null 2>&1; then timeout "$seconds" "$@"
    else
        "$@" &
        local child_pid=$!
        (sleep "$seconds"; kill -9 "$child_pid" 2>/dev/null) &
        local killer_pid=$!
        local exit_code=0
        wait "$child_pid" 2>/dev/null || exit_code=$?
        kill "$killer_pid" 2>/dev/null || true
        return $exit_code
    fi
}

function run_supervised_task() {
    local timeout_seconds="$1"
    local workdir="$2"
    local command_str="$3"

    exec "$PYTHON_CMD" - "$timeout_seconds" "$workdir" "$command_str" <<'PY'
import os
import signal
import subprocess
import sys
import time

timeout_seconds = int(sys.argv[1])
workdir = sys.argv[2]
command_str = sys.argv[3]
child = None


def terminate_group(sig, grace_seconds=1.0):
    if child is None:
        return
    try:
        os.killpg(child.pid, sig)
    except ProcessLookupError:
        return

    deadline = time.time() + grace_seconds
    while time.time() < deadline:
        try:
            os.killpg(child.pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.05)

    if sig != signal.SIGKILL:
        try:
            os.killpg(child.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def handle_signal(signum, _frame):
    terminate_group(signal.SIGTERM)
    sys.exit(128 + signum)


signal.signal(signal.SIGTERM, handle_signal)
signal.signal(signal.SIGINT, handle_signal)

child = subprocess.Popen(
    ["bash", "-c", command_str],
    cwd=workdir,
    preexec_fn=os.setsid,
)

try:
    if timeout_seconds > 0:
        return_code = child.wait(timeout=timeout_seconds)
    else:
        return_code = child.wait()
except subprocess.TimeoutExpired:
    terminate_group(signal.SIGTERM)
    sys.exit(124)

terminate_group(signal.SIGTERM, grace_seconds=0.5)
sys.exit(return_code)
PY
}

function run_supervised_command_in_dir() {
    local timeout_seconds="$1"
    local workdir="$2"
    shift 2

    local command_str
    command_str="$(shell_join "$@")"
    run_supervised_task "$timeout_seconds" "$workdir" "$command_str" &
    local pid=$!
    wait "$pid"
}

function register_temp_artifact() {
    TEMP_ARTIFACTS+=("$1")
}

function unregister_temp_artifact() {
    local target="$1"
    local remaining=()
    local artifact
    for artifact in "${TEMP_ARTIFACTS[@]}"; do
        if [ "$artifact" != "$target" ]; then
            remaining+=("$artifact")
        fi
    done
    if [ "${#remaining[@]}" -gt 0 ]; then
        TEMP_ARTIFACTS=("${remaining[@]}")
    else
        TEMP_ARTIFACTS=()
    fi
}

function run_supervised_capture_command_in_dir() {
    local result_var="$1"
    local timeout_seconds="$2"
    local workdir="$3"
    shift 3

    local command_str
    local output_file
    local pid
    local exit_code
    local captured=""
    command_str="$(shell_join "$@")"
    output_file="$(mktemp "${TMPDIR:-/tmp}/agent-manager-capture.XXXXXX")" || return 1
    register_temp_artifact "$output_file"

    run_supervised_task "$timeout_seconds" "$workdir" "$command_str" > "$output_file" &
    pid=$!
    wait "$pid"
    exit_code=$?

    if [ -f "$output_file" ]; then
        captured="$(cat "$output_file")"
    fi
    unregister_temp_artifact "$output_file"
    rm -f "$output_file"

    printf -v "$result_var" '%s' "$captured"
    return "$exit_code"
}

function update_latest_log_pointer() {
    mkdir -p "$(dirname "$TEMP_LOG")"
    : > "$TEMP_LOG"
    printf '%s\n' "$TEMP_LOG" > "$LATEST_LOG_POINTER"
    chmod 600 "$TEMP_LOG" "$LATEST_LOG_POINTER" 2>/dev/null || true
    ln -sfn "$TEMP_LOG" "$LEGACY_LOG_PATH" 2>/dev/null || true
}

function detect_agents() {
    echo -e "${HDR_BLUE}${BOLD}2. 环境探测与健康自检 (Environment Health Check)${NC} ${BORDER_GRAY}[OS: $OS_TYPE]${NC}"
    $PYTHON_CMD -c "
import sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.json_config import ensure_mapping_value_types, get_typed_mapping_section
from src.manage_runtime_probe import (
    SERENA_MIN_NODE_MAJOR,
    ProviderInventory,
    collect_active_mcp_target_agents,
    collect_active_rule_agents,
    collect_active_skill_target_agents,
    detect_all_providers,
    detect_serena_readiness,
    detect_tool_readiness,
    is_agent_installed,
    load_json,
    rule_target_agent_id,
)

try:
    inventory = ProviderInventory()
    cfg = load_json('$CONFIG_FILE', strict=True, missing_ok=False)
    cfg['agents'] = canonicalize_agent_mapping(get_typed_mapping_section(cfg, 'agents', 'manager.json', (dict,), '对象'))
    def_a = canonicalize_agent_mapping(
        ensure_mapping_value_types(
            load_json('$ROOT_DIR/config/agents.json', strict=True, missing_ok=False),
            'agents.json',
            (dict,),
            '对象',
        )
    )
    tool_defs = get_typed_mapping_section(
        load_json('$ROOT_DIR/config/tools.json', strict=True, missing_ok=False),
        'tools',
        'tools.json',
        (dict,),
        '对象',
    )
    for k, v in cfg.get('agents', {}).items():
        if not v.get('enabled', True): continue
        d = def_a.get(k, {})
        label = get_agent_display_name(k, d)
        all_act = detect_all_providers(d.get('bin',''), d, inventory=inventory)
        stat = '${SUCCESS_GREEN}✓${NC}' if all_act else '${PENDING_GOLD}○${NC}'
        print(f'   {stat} {label:20} (安装源:{\"/\".join(all_act) if all_act else \"未发现\"})')
    for k, v in cfg.get('tools', {}).items():
        if v.get('enabled', True):
            act = detect_tool_readiness(k, tool_defs.get(k, {}), '$ROOT_DIR', inventory=inventory)
            stat = '${SUCCESS_GREEN}⚒${NC}' if act else '${PENDING_GOLD}⚠${NC}'
            print(f'   {stat} {k:20} (工具就绪:{\"是\" if act else \"否\"})')
    serena_policy = cfg.get('mcp', {}).get('serena')
    if serena_policy and serena_policy.get('enabled', True):
        uvx_ok, node_major = detect_serena_readiness(inventory=inventory)
        node_ok = node_major is not None and node_major >= SERENA_MIN_NODE_MAJOR
        stat = '${SUCCESS_GREEN}⚒${NC}' if (uvx_ok and node_ok) else '${PENDING_GOLD}⚠${NC}'
        node_txt = f'是(v{node_major})' if node_ok else (f'否(v{node_major})' if node_major is not None else '否(未发现)')
        print(f'   {stat} {\"serena\":20} (uvx:{\"是\" if uvx_ok else \"否\"} / Node≥{SERENA_MIN_NODE_MAJOR}:{node_txt})')
except ValueError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(1)
"
    echo ""
}

function doctor_check() {
    local mode="${1:-text}"
    local json_report
    if ! json_report="$($PYTHON_CMD -m src.manage_doctor --root-dir "$ROOT_DIR" --config-file "$CONFIG_FILE" --python-cmd "$PYTHON_CMD" --format json)"; then
        true
    fi

    read -r DOCTOR_STATUS DOCTOR_WARNING_OPTIONAL_TOOLS DOCTOR_WARNING_UNMATERIALIZED <<EOF
$($PYTHON_CMD - "$json_report" <<'PY'
import json
import sys

report = json.loads(sys.argv[1])
summary = report.get("summary", {})
categories = summary.get("warning_categories", {})
print(
    report.get("status", "error"),
    categories.get("optional_tools", 0),
    categories.get("unmaterialized", 0),
)
PY
)
EOF

    case "$mode" in
        json)
            printf '%s\n' "$json_report"
            ;;
        silent)
            ;;
        *)
            "$PYTHON_CMD" -m src.manage_doctor --root-dir "$ROOT_DIR" --config-file "$CONFIG_FILE" --python-cmd "$PYTHON_CMD" --format text
            ;;
    esac

    [ "$DOCTOR_STATUS" != "error" ]
}

function run_task_with_progress() {
    local title="$1"
    local success_msg="$2"
    local err_msg="$3"
    shift 3

    status_line "RUNNING" "$title..."
    local start_line=0
    if [ -f "$TEMP_LOG" ]; then
        start_line=$(wc -l < "$TEMP_LOG" | tr -d ' ')
    fi
    local start_ts
    start_ts=$(date +%s)
    local last_display=""
    local last_display_key=""
    local last_raw_msg=""
    local last_msg=""
    local last_detail_lines=0

    "$@" >> "$TEMP_LOG" 2>&1 &
    local pid=$!

    while kill -0 $pid 2>/dev/null; do
        local elapsed
        elapsed=$(( $(date +%s) - start_ts ))
        local current_display=""
        local current_key=""
        local detail_output=""
        local current_detail_lines=0

        if [ -s "$TEMP_LOG" ]; then
            local next_line=$((start_line + 1))
            local raw_msg=""
            raw_msg=$(tail -n +"$next_line" "$TEMP_LOG" 2>/dev/null | tail -n 1)
            if [ -n "$raw_msg" ] && [ "$raw_msg" != "$last_raw_msg" ]; then
                last_raw_msg="$raw_msg"
                last_msg=$(format_progress_tail "$raw_msg")
            fi
            if [ -n "$last_msg" ]; then
                current_display="$title: $last_msg · 已运行 ${elapsed}s"
                current_key="$title:$last_msg"
            else
                current_display="$title... · 已运行 ${elapsed}s"
                current_key="$current_display"
            fi

            detail_output=$(tail -n +"$next_line" "$TEMP_LOG" 2>/dev/null | tail -n 5 | tr -d '\r' | sed -E $'s/\033\\[[0-9;]*[a-zA-Z]//g' | cut -c 1-100 | sed -e $'s/^/      │ \033[90m/' -e $'s/$/\033[0m/')
            if [ -n "$detail_output" ]; then
                current_detail_lines=$(echo -n "$detail_output" | grep -c '^' || true)
            fi
        else
            current_display="$title... · 已运行 ${elapsed}s"
            current_key="$current_display"
        fi

        if [ "$last_detail_lines" -gt 0 ]; then
            printf "\033[%dA" "$last_detail_lines"
        fi

        status_line "RUNNING" "$current_display"
        printf "\033[J"

        if [ "$current_detail_lines" -gt 0 ]; then
            printf "\n%s" "$detail_output"
        fi

        last_detail_lines="$current_detail_lines"
        sleep 0.5
    done

    if [ "$last_detail_lines" -gt 0 ]; then
        printf "\033[%dA" "$last_detail_lines"
        printf "\033[J"
    fi

    if wait "$pid"; then
        status_line "DONE" "$success_msg"
        return 0
    else
        local exit_code=$?
        status_line "ERROR" "$err_msg"
        echo "FAILED: $title" >> "$TEMP_LOG"
        if [ -f "$TEMP_LOG" ]; then
            local recent_errors=""
            recent_errors=$(grep -E "ERROR|安装失败|工具失败|TIMEOUT|failed with exit code" "$TEMP_LOG" | tail -n 3)
            if [ -n "$recent_errors" ]; then
                echo -e "${ERROR_RED}详情:${NC}"
                echo "$recent_errors" | sed 's/^/  /'
            fi
        fi
        return "$exit_code"
    fi
}

function show_summary() {
    echo -e "${HDR_BLUE}${BOLD}4. 最终赋能详情汇总 (Deployment Architecture)${NC}"
    echo -e "${BORDER_GRAY}----------------------------------------------------------------${NC}"
    if [ ! -f "$TEMP_LOG" ]; then echo "   无运行记录。"; return; fi
    local clean_log=$(sed -E 's/^[0-9:-]+ [0-9:-]+ \[INFO\] //g; s/^INFO: //g' "$TEMP_LOG")
    echo -e "${BOLD}${HDR_BLUE}[ 生命周期与配置同步 ]${NC}"
    echo "$clean_log" | grep -E "状态对齐|更新 MCP" | sed 's/\[AGENT: \(.*\)\]/  ● \1/g' | sort -u || echo "   无变动。"
    echo -e "\n${BOLD}${HDR_BLUE}[ 规则与工具 ]${NC}"
    echo "$clean_log" | grep -E "同步完成|安装成功" | sed 's/\[AGENT: Rules\] //g; s/\[TOOL: \(.*\)\]/  ⚒ \1/g' | sort -u || echo "   无变动。"
    echo -e "${BORDER_GRAY}----------------------------------------------------------------${NC}"
    echo -e "${HDR_BLUE}${BOLD}5. 自动健康验证 (Post-Sync Verification)${NC}"
    if [ "$TASK_FAILURES" -ne 0 ]; then
        echo -e "   ${ERROR_RED}${BOLD}HEALTH CHECK SKIPPED${NC}"
        echo -e "   ${ERROR_RED}存在失败任务，未宣告健康状态。${NC}"
        echo ""
        return
    fi
    if doctor_check silent; then
        case "$DOCTOR_STATUS" in
            healthy) echo -e "   ${SUCCESS_GREEN}${BOLD}CERTIFIED HEALTHY${NC}" ;;
            warning)
                echo -e "   ${PENDING_GOLD}${BOLD}HEALTH CAUTION${NC}"
                local warning_parts=()
                if [ "${DOCTOR_WARNING_OPTIONAL_TOOLS:-0}" -gt 0 ]; then
                    warning_parts+=("可选工具未就绪 ${DOCTOR_WARNING_OPTIONAL_TOOLS} 项")
                fi
                if [ "${DOCTOR_WARNING_UNMATERIALIZED:-0}" -gt 0 ]; then
                    warning_parts+=("托管产物尚未生成 ${DOCTOR_WARNING_UNMATERIALIZED} 项")
                fi
                if [ ${#warning_parts[@]} -gt 0 ]; then
                    local joined_warning_parts=""
                    local part
                    for part in "${warning_parts[@]}"; do
                        if [ -n "$joined_warning_parts" ]; then
                            joined_warning_parts="${joined_warning_parts}；"
                        fi
                        joined_warning_parts="${joined_warning_parts}${part}"
                    done
                    echo -e "   ${PENDING_GOLD}${joined_warning_parts}${NC}"
                fi
                ;;
            *) echo -e "   ${SUCCESS_GREEN}${BOLD}CERTIFIED HEALTHY${NC}" ;;
        esac
    else
        echo -e "   ${ERROR_RED}${BOLD}HEALTH WARNING${NC}"
    fi
    echo ""
}

function begin_workflow() {
    local title="$1"
    if [ -t 1 ] && command -v clear >/dev/null 2>&1 && [ -n "${TERM:-}" ] && [ "${TERM:-}" != "dumb" ]; then
        clear
    fi
    echo -e "${HDR_BLUE}${BOLD}${title}${NC}\n"
}

function shell_join() {
    local rendered=""
    printf -v rendered '%q ' "$@"
    printf '%s' "${rendered% }"
}

function run_task_in_dir() {
    local title="$1"
    local success_msg="$2"
    local err_msg="$3"
    local timeout_seconds="$4"
    local workdir="$5"
    shift 5

    local command_str
    command_str="$(shell_join "$@")"
    run_task_with_progress "$title" "$success_msg" "$err_msg" run_supervised_task "$timeout_seconds" "$workdir" "$command_str"
}

function begin_pipeline_execution() {
    detect_agents
    echo -e "${HDR_BLUE}${BOLD}3. 任务流水线 (Tasks)${NC}"
    update_latest_log_pointer
    TASK_FAILURES=0
}

function run_pipeline_step() {
    if ! "$@"; then
        TASK_FAILURES=1
    fi
}

function finish_pipeline_execution() {
    show_summary
    [ "$TASK_FAILURES" -eq 0 ] || exit 1
}

# 在管线中途设一道闸：前一段失败就地收尾退出，不再执行后续步骤。
# 用于 update 这类「A + B」复合管线，保证失败语义与顺序执行 A && B 一致。
function abort_pipeline_if_failed() {
    local reason="$1"
    if [ "$TASK_FAILURES" -ne 0 ]; then
        status_line "ERROR" "$reason"
        finish_pipeline_execution
    fi
}

function maybe_init_secrets() {
    echo -e "${HDR_BLUE}${BOLD}1. 凭证仓库检查 (Secrets Check)${NC}"
    if [ -d "$ROOT_DIR/secrets/.git" ]; then
        if [ -n "$OFFLINE_FLAG" ]; then
            status_line "SKIP" "离线模式，跳过 secrets 拉取"
        elif [ -n "$DRY_RUN_FLAG" ]; then
            status_line "SKIP" "空转模式，跳过 secrets 拉取"
        else
            status_line "INFO" "拉取最新凭证..."
            if ! run_supervised_command_in_dir 600 "$ROOT_DIR" git -C "$ROOT_DIR/secrets" pull --ff-only; then
                echo -e "${ERROR_RED}⚠️  secrets 拉取失败，将使用本地已有凭证${NC}"
            fi
        fi
    else
        if [ -n "$OFFLINE_FLAG" ]; then
            status_line "SKIP" "离线模式，跳过 secrets 检查"
        elif [ -n "$DRY_RUN_FLAG" ]; then
            status_line "SKIP" "空转模式，跳过 secrets 检查"
        else
            if [ -x "$ROOT_DIR/scripts/init-secrets.sh" ]; then
                run_supervised_command_in_dir 600 "$ROOT_DIR" bash "$ROOT_DIR/scripts/init-secrets.sh" || {
                    echo -e "${ERROR_RED}⚠️  secrets 初始化失败，继续执行但 MCP 可能无法正常工作${NC}"
                }
            fi
        fi
    fi
    echo ""
}

function run_agent_lifecycle_task() {
    local extra_flag="${1:-}"
    local args=(sync_agents.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/agents.json" --yes)
    if [ -n "$extra_flag" ]; then args+=("$extra_flag"); fi
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$OFFLINE_FLAG" ]; then args+=("$OFFLINE_FLAG"); fi
    if [ -z "$DRY_RUN_FLAG" ]; then args+=(--async-actions); fi
    if [ -n "$WAIT_AGENT_LIFECYCLE_FLAG" ]; then args+=("$WAIT_AGENT_LIFECYCLE_FLAG"); fi
    run_task_in_dir "对齐 Agent 生命周期" "生命周期对齐完成" "生命周期失败" 600 "$ROOT_DIR/agent-lifecycle" "$PYTHON_CMD" "${args[@]}"
}

function run_shell_aliases_sync_task() {
    local args=("$ROOT_DIR/scripts/claude/sync_shell_aliases.sh")
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    run_task_in_dir "同步 Shell Alias" "Shell alias 同步完成" "Shell alias 同步失败" 60 "$ROOT_DIR" bash "${args[@]}"
}

function run_tools_sync_task() {
    local args=(sync_tools.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/tools.json" --yes)
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$OFFLINE_FLAG" ]; then args+=("$OFFLINE_FLAG"); fi
    if [ -n "$FORCE_REFRESH_FLAG" ]; then args+=("$FORCE_REFRESH_FLAG"); fi
    run_task_in_dir "同步全局工具" "工具链同步完成" "工具失败" 600 "$ROOT_DIR/agent-tool" "$PYTHON_CMD" "${args[@]}"
}

function browser_us_sync_enabled() {
    local override="${AGENT_MANAGER_BROWSER_US_SYNC_ENABLED:-}"
    local normalized_override
    normalized_override="$(printf '%s' "$override" | tr '[:upper:]' '[:lower:]')"
    case "$normalized_override" in
        "")
            ;;
        1|true|yes|on|enabled)
            return 0
            ;;
        0|false|no|off|disabled)
            return 1
            ;;
        *)
            echo -e "${ERROR_RED}错误: AGENT_MANAGER_BROWSER_US_SYNC_ENABLED 必须是 true/false、1/0、yes/no 或 on/off${NC}" >&2
            return 2
            ;;
    esac

    "$PYTHON_CMD" - "$CONFIG_FILE" <<'PY'
import json
import sys
from pathlib import Path

config_path = Path(sys.argv[1])
try:
    data = json.loads(config_path.read_text(encoding="utf-8"))
except Exception as exc:
    print(f"manager.json 无效: {exc}", file=sys.stderr)
    raise SystemExit(2)

settings = data.get("browser_us_sync", {})
if settings is None:
    settings = {}
if not isinstance(settings, dict):
    print("manager.json.browser_us_sync 必须是对象", file=sys.stderr)
    raise SystemExit(2)

enabled = settings.get("enabled", True)
if not isinstance(enabled, bool):
    print("manager.json.browser_us_sync.enabled 必须是布尔值", file=sys.stderr)
    raise SystemExit(2)

raise SystemExit(0 if enabled else 1)
PY
}

function run_browser_us_sync_task() {
    if browser_us_sync_enabled; then
        :
    else
        local status="$?"
        if [ "$status" -eq 1 ]; then
            status_line "SKIP" "浏览器 US 同步已禁用"
            return 0
        fi
        return "$status"
    fi

    local args=(scripts/sync_browser_google_us.py)
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$OFFLINE_FLAG" ]; then args+=("$OFFLINE_FLAG"); fi
    run_task_in_dir "同步浏览器 Google US 前置状态" "浏览器 US 状态同步完成" "浏览器 US 同步失败" 120 "$ROOT_DIR" "$PYTHON_CMD" "${args[@]}"
}

function list_active_rule_targets() {
    ROOT_DIR="$ROOT_DIR" CONFIG_FILE="$CONFIG_FILE" "$PYTHON_CMD" - <<'PY'
import os
import sys
from pathlib import Path

root = Path(os.environ["ROOT_DIR"])
config_path = os.environ["CONFIG_FILE"]
sys.path.insert(0, str(root))

from src.agent_ids import canonicalize_agent_mapping
from src.json_config import get_typed_mapping_section
from src.manage_runtime_probe import (
    collect_active_rule_agents,
    load_json,
    rule_target_agent_id,
)

try:
    cfg = load_json(config_path, strict=True, missing_ok=False)
    cfg["agents"] = canonicalize_agent_mapping(get_typed_mapping_section(cfg, "agents", "manager.json", (dict,), "对象"))
    skill_cfg_raw = load_json(root / "config" / "skills.json", strict=True, missing_ok=False)
    skill_cfg = {
        "repositories": get_typed_mapping_section(skill_cfg_raw, "repositories", "skills.json", (dict,), "对象"),
        "agents": get_typed_mapping_section(skill_cfg_raw, "agents", "skills.json", (str, dict), "字符串或对象"),
    }
    mcp_cfg_raw = load_json(root / "config" / "mcp.json", strict=True, missing_ok=False)
    mcp_cfg = {
        "servers": get_typed_mapping_section(mcp_cfg_raw, "servers", "mcp.json", (dict,), "对象"),
        "agent_paths": get_typed_mapping_section(mcp_cfg_raw, "agent_paths", "mcp.json", (str, dict), "字符串或对象"),
    }
except ValueError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(1)

active_rule_agents = collect_active_rule_agents(cfg, skill_cfg, mcp_cfg)
manifest_path = root / "config" / "rules-targets.list"
if not manifest_path.exists():
    print("rules-targets.list 缺失", file=sys.stderr)
    sys.exit(1)

for raw_line in manifest_path.read_text(encoding="utf-8").splitlines():
    line = raw_line.strip()
    if not line or line.startswith("#"):
        continue
    parts = [part.strip() for part in line.split("|")]
    if len(parts) < 4:
        print(f"rules-targets.list 非法条目: {raw_line}", file=sys.stderr)
        sys.exit(1)
    if "{{REPO_ROOT}}" in parts[3] or rule_target_agent_id(parts[0]) in active_rule_agents:
        print(parts[0])
PY
}

function run_rules_sync_task() {
    local args=(./scripts/sync-agent-rules.sh)
    local target_names
    if ! target_names="$(list_active_rule_targets)"; then
        return 1
    fi
    if [ -z "$target_names" ]; then
        status_line "SKIP" "当前策略未启用任何 Rules 目标"
        return 0
    fi
    while IFS= read -r target_name; do
        [ -n "$target_name" ] && args+=(--target "$target_name")
    done <<< "$target_names"
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    run_task_in_dir "同步 Agent Rules" "规则分发完成" "规则同步失败" 600 "$ROOT_DIR/agent-rule" "${args[@]}"
}

function run_skills_link_task() {
    local force_offline="${1:-false}"
    local args=(link_skills.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/skills.json" --yes)
    if [ "$force_offline" = "true" ] || [ -n "$OFFLINE_FLAG" ]; then args+=(--offline); fi
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$FORCE_REFRESH_FLAG" ]; then args+=("$FORCE_REFRESH_FLAG"); fi
    run_task_in_dir "链接专业技能" "技能链接完成" "技能链接失败" 600 "$ROOT_DIR/agent-skill" "$PYTHON_CMD" "${args[@]}"
}

function run_mcp_sync_task() {
    local prepare_runtime="${1:-false}"
    local force_offline="${2:-false}"
    local title="注入 MCP 配置"
    local args=(sync_mcp.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/mcp.json")

    if [ "$prepare_runtime" = "true" ]; then
        title="准备并注入 MCP 配置"
        args+=(--prepare-runtime)
    fi
    if [ "$force_offline" = "true" ] || [ -n "$OFFLINE_FLAG" ]; then args+=(--offline); fi
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$FORCE_REFRESH_FLAG" ]; then args+=("$FORCE_REFRESH_FLAG"); fi

    run_task_in_dir "$title" "MCP 注入完成" "MCP 失败" 600 "$ROOT_DIR/agent-mcp" "$PYTHON_CMD" "${args[@]}"
}

function run_skill_reference_audit_warn() {
    # 内容层失效引用检测(非致命):在结构收敛(软链/pin/规则分发)之外,提醒 rules 或
    # SKILL.md 是否点名了不存在的 skill。只警告、不阻断收敛、不改写内容;dry-run 跳过
    # 以免扰动 preview/smoke 契约。fail-close 由 instructions:check 的测试承担。
    [ -n "$DRY_RUN_FLAG" ] && return 0
    local out
    if ! out="$("$PYTHON_CMD" "$ROOT_DIR/scripts/audit_skill_references.py" --root-dir "$ROOT_DIR" 2>&1)"; then
        echo -e "${ERROR_RED}⚠️  skill 引用审计(仅警告,未阻断同步):${NC}"
        printf '%s\n' "$out" | sed 's/^/    /'
    fi
}

function run_claude_hooks_sync_task() {
    local args=(./scripts/sync-claude-hooks.py)
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    run_task_in_dir "同步 Claude hooks" "Claude hooks 同步完成" "Claude hooks 同步失败" 600 "$ROOT_DIR/agent-rule" "$PYTHON_CMD" "${args[@]}"
}

function run_codex_hooks_sync_task() {
    local args=(./scripts/sync-codex-hooks.py)
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    run_task_in_dir "同步 Codex hooks" "Codex hooks 同步完成" "Codex hooks 同步失败" 600 "$ROOT_DIR/agent-rule" "$PYTHON_CMD" "${args[@]}"
}

# 受管组件的收敛步骤序列。update / init 两条管线只负责组合它们，
# 避免同一串步骤在仓库里各写一份后逐渐漂移。
function run_apply_steps() {
    run_pipeline_step run_shell_aliases_sync_task
    run_pipeline_step run_tools_sync_task
    run_pipeline_step run_browser_us_sync_task
    run_pipeline_step run_rules_sync_task
    run_pipeline_step run_skills_link_task false
    run_pipeline_step run_mcp_sync_task true false
    run_pipeline_step run_claude_hooks_sync_task
    run_pipeline_step run_codex_hooks_sync_task
    run_skill_reference_audit_warn
}

# update：把所有受管组件分发/收敛到本机；不安装、也不升级 Agent 软件。
function run_update_pipeline() {
    begin_workflow "🔄 Agent Manager 更新并收敛启动"
    maybe_init_secrets
    begin_pipeline_execution
    run_apply_steps
    finish_pipeline_execution
}

function run_agents_pipeline() {
    local agent_extra_flags="$1"
    begin_workflow "🔄 Agent Manager 环境对齐启动"
    begin_pipeline_execution
    run_pipeline_step run_agent_lifecycle_task "$agent_extra_flags"
    finish_pipeline_execution
}

# init = Agent 生命周期(装缺失 + 升级已装) + update 的全量分发。
function run_init_pipeline() {
    begin_workflow "🚀 Agent Manager 全量初始化启动"
    maybe_init_secrets
    begin_pipeline_execution
    run_pipeline_step run_agent_lifecycle_task ""
    run_apply_steps
    finish_pipeline_execution
}

function print_help() {
    echo -e "${HDR_BLUE}${BOLD}Agent Manager 帮助手册${NC}\n"
    echo -e "直接执行 ${BOLD}./manage.sh${NC} 时默认显示本帮助，不会触发同步。\n"
    echo -e "${BOLD}${HDR_BLUE}=== 全局参数 (Flags) ===${NC}"
    echo -e "  ${PENDING_GOLD}--dry-run${NC}       空转预览，仅输出计划动作，不改本地/远端状态"
    echo -e "  ${PENDING_GOLD}--offline${NC}       离线模式，禁止远程拉取/安装（如 ${BOLD}--offline update${NC}）"
    echo -e "  ${PENDING_GOLD}-f, --force-refresh${NC} 探测失败时仍强制远端刷新"
    echo -e "  ${PENDING_GOLD}--wait-agent-lifecycle${NC} 等待 Agent 生命周期后台安装/升级结束\n"
    echo -e "${BOLD}${HDR_BLUE}=== 命令 (Commands) ===${NC}"
    echo -e "  ${SUCCESS_GREEN}update${NC}         ${BOLD}日常最常用${NC}：把所有受管组件收敛到本机（secrets、shell alias、tools、browser、rules、skills、MCP、hooks）；不安装、也不升级 Agent 软件"
    echo -e "  ${SUCCESS_GREEN}init${NC}           新机器初始化或全量收敛：安装缺失 Agent + 升级已装 Agent + \`update\`"
    echo -e "  ${SUCCESS_GREEN}agents:update${NC}  升级/修复已装 Agent 到上游最新（agents.json 不锁版本）；不安装缺失 Agent"
    echo -e "  ${SUCCESS_GREEN}doctor${NC}         深度资源校验、配置自检与环境排错（追加 --json 输出机器可读报告）"
    echo -e "  ${SUCCESS_GREEN}mcp:status${NC}     MCP server × agent 注入状态（引擎契约 envelope，须追加 --json；--dry-run/--offline 透传为 mode）"
    echo -e "  ${SUCCESS_GREEN}*:status${NC}       域状态命令族：agents/mcp/skills/rules/hooks/tools/aliases/secrets（引擎契约 envelope，须追加 --json）"
    echo -e "  ${SUCCESS_GREEN}*:plan${NC}         域收敛计划预览（agents/mcp/skills/rules/hooks/tools/aliases；secrets 只读无 plan）"
    echo -e "  ${SUCCESS_GREEN}*:apply${NC}        域收敛写路径（agents/mcp/skills/rules/hooks/tools/aliases；复用既有同步器；非 dry-run 前自动快照；agents 卸载须显式 --allow-uninstall）"
    echo -e "  ${SUCCESS_GREEN}system:converge${NC} 全管线收敛（= update 管线语义；--dry-run 预演）"
    echo -e "  ${SUCCESS_GREEN}system:audit${NC}   全系统体检的契约形态（doctor --json 的 envelope 版）"
    echo -e "  ${SUCCESS_GREEN}system:refresh-upstream${NC} 上游新鲜度探测并写探测缓存（network 级，offline 拒绝；--only <元素id> 单元素刷新；status 命令只读缓存、零网络）"
    echo -e "  ${SUCCESS_GREEN}snapshots:list${NC} / ${SUCCESS_GREEN}snapshots:restore --run <id>${NC} 运行快照清单与回滚（restore 前会再快照当前态）"
    echo -e "  ${SUCCESS_GREEN}runs:list${NC} / ${SUCCESS_GREEN}runs:get --run <id>${NC} 执行历史与按次日志读取（--lines 调尾部行数）"
    echo -e "  ${SUCCESS_GREEN}policy:get${NC} / ${SUCCESS_GREEN}policy:set${NC} 三层策略视图（baseline⊕synced⊕local 原文+effective+层归属）与分层写入（仅 synced/local，疑似明文门禁）"
    echo -e "  ${SUCCESS_GREEN}profile:status${NC} 个性化配置同步状态（共享层本地状态 + 远端比对 clean/ahead/behind/diverged/uninitialized；须追加 --json；--offline 只报本地）"
    echo -e "  ${SUCCESS_GREEN}profile:pull${NC}   拉取远端共享层并三向合并写回 synced 层（network 级，offline 拒绝；冲突保留本地值并按 key 报告）"
    echo -e "  ${SUCCESS_GREEN}profile:push${NC}   发布本地共享层到同步后端（push 前疑似明文扫描硬门禁；--dry-run 只出计划不推送；后端地址只经 PROFILE_SYNC_REPO_URL 或 manager.local.json 提供）"
    echo -e "  ${SUCCESS_GREEN}help${NC}           显示本帮助\n"
}

case "$cmd" in
    init)
        if [ -z "$WAIT_AGENT_LIFECYCLE_FLAG" ]; then
            WAIT_AGENT_LIFECYCLE_FLAG="--wait-async-actions"
        fi
        run_init_pipeline
        ;;
    update)
        run_update_pipeline
        ;;
    agents:update)
        if [ -z "$WAIT_AGENT_LIFECYCLE_FLAG" ]; then
            WAIT_AGENT_LIFECYCLE_FLAG="--wait-async-actions"
        fi
        run_agents_pipeline "--no-install"
        ;;
    doctor)
        if [ "${CMD_ARGS[0]:-}" = "--json" ]; then
            doctor_check json
        else
            doctor_check
        fi
        ;;
    profile:status|profile:pull|profile:push)
        # 个性化配置同步引擎（§7.2，git 后端，agent-profile/sync_profile.py）；
        # 契约与退出码同引擎命令族。profile:status 经 *:status 通配已享锁豁免，
        # pull/push 持锁（network 级写动作）。
        profile_args=("$cmd" --synced-config "$SYNCED_CONFIG" --local-config "$LOCAL_CONFIG")
        if [ -n "$DRY_RUN_FLAG" ]; then
            profile_args+=(--dry-run)
        fi
        if [ -n "$OFFLINE_FLAG" ]; then
            profile_args+=(--offline)
        fi
        if [ "${#CMD_ARGS[@]}" -gt 0 ]; then
            profile_args+=("${CMD_ARGS[@]}")
        fi
        AGENT_MANAGER_RUN_ID="$RUN_ID" \
        AGENT_MANAGER_LOG_FILE="$TEMP_LOG" \
        AGENT_MANAGER_LOCK_HELD="$LOCK_ACQUIRED" \
        AGENT_MANAGER_LOCK_PID="$$" \
            "$PYTHON_CMD" "$ROOT_DIR/agent-profile/sync_profile.py" "${profile_args[@]}"
        ;;
    *:status|*:plan|*:apply|system:audit|system:refresh-upstream|system:converge|snapshots:list|snapshots:restore|runs:list|runs:get|policy:get|policy:set)
        # 引擎契约命令族（contract/engine-v1）：envelope JSON 走 stdout，退出码 0/1/2 由引擎决定。
        engine_args=("$cmd" --config "$CONFIG_FILE" --python-cmd "$PYTHON_CMD")
        if [[ "$cmd" == mcp:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/mcp.json")
        elif [[ "$cmd" == agents:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/agents.json")
        elif [[ "$cmd" == tools:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/tools.json")
        elif [[ "$cmd" == rules:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/rules-targets.list")
        elif [[ "$cmd" == hooks:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/claude-hooks.json" --def-file "$ROOT_DIR/config/codex-hooks.json")
        elif [[ "$cmd" == aliases:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/shell-aliases.json")
        elif [[ "$cmd" == skills:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/skills.json")
        elif [[ "$cmd" == secrets:* ]]; then
            engine_args+=(--def-file "$ROOT_DIR/config/mcp.json")
        fi
        if [ -n "$DRY_RUN_FLAG" ]; then
            engine_args+=(--dry-run)
        fi
        if [ -n "$OFFLINE_FLAG" ]; then
            engine_args+=(--offline)
        fi
        if [ "${#CMD_ARGS[@]}" -gt 0 ]; then
            engine_args+=("${CMD_ARGS[@]}")
        fi
        # Rust 切片路由（Phase 3）：mcp 域优先走 engine-rs（Python 内核保留作
        # fallback；退役待 Phase 3 末 CI 全绿）。AGENT_MANAGER_ENGINE_RS 可显式指定 binary。
        engine_cmd=("$PYTHON_CMD" "$ROOT_DIR/src/engine_v1.py")
        if [[ "$cmd" == "mcp:status" || "$cmd" == "mcp:plan" ]]; then
            rust_engine=""
            if [ -n "${AGENT_MANAGER_ENGINE_RS:-}" ] && [ -x "${AGENT_MANAGER_ENGINE_RS}" ]; then
                rust_engine="$AGENT_MANAGER_ENGINE_RS"
            elif [ -x "$ROOT_DIR/engine-rs/target/release/agent-manager-engine" ]; then
                rust_engine="$ROOT_DIR/engine-rs/target/release/agent-manager-engine"
            elif [ -x "$ROOT_DIR/engine-rs/target/debug/agent-manager-engine" ]; then
                rust_engine="$ROOT_DIR/engine-rs/target/debug/agent-manager-engine"
            fi
            if [ -n "$rust_engine" ]; then
                engine_cmd=("$rust_engine")
                echo -e "${BORDER_GRAY}[engine] rust: $rust_engine${NC}" >&2
            fi
        fi
        AGENT_MANAGER_RUN_ID="$RUN_ID" \
        AGENT_MANAGER_LOG_FILE="$TEMP_LOG" \
        AGENT_MANAGER_LOCK_HELD="$LOCK_ACQUIRED" \
        AGENT_MANAGER_LOCK_PID="$$" \
        AGENT_MANAGER_ROOT_DIR="$ROOT_DIR" \
            "${engine_cmd[@]}" "${engine_args[@]}"
        ;;
    help|-h|--help|-help)
        print_help ;;
    *) echo -e "${ERROR_RED}错误: 未知命令 '$cmd'${NC}"; exit 1 ;;
esac
