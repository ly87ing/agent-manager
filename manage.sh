#!/usr/bin/env bash

set -euo pipefail

# 颜色定义
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

# 获取脚本所在根目录绝对路径
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
USER_NAME=$(whoami)
RUN_ID="$(date +%Y%m%d%H%M%S)_$$"
LOG_PREFIX="${AGENT_MANAGER_LOG_PREFIX:-/tmp/agent_manager_sync_${USER_NAME}}"
TEMP_LOG="${LOG_PREFIX}_${RUN_ID}.log"
LATEST_LOG_POINTER="${AGENT_MANAGER_LATEST_LOG_POINTER:-${LOG_PREFIX}.latest}"
LEGACY_LOG_PATH="${AGENT_MANAGER_LEGACY_LOG_PATH:-${LOG_PREFIX}.log}"
LOCK_FILE="${AGENT_MANAGER_LOCK_FILE:-/tmp/agent_manager_${USER_NAME}.lock}"
LOCK_DIR="${LOCK_FILE}.d"
LOCK_PID_FILE="${LOCK_DIR}/pid"
# 基线策略（仓库提供）
BASE_CONFIG="${AGENT_MANAGER_BASE_CONFIG:-$ROOT_DIR/config/manager.json}"
# 本地覆盖（不参与 Git）
LOCAL_CONFIG="${AGENT_MANAGER_LOCAL_CONFIG:-$ROOT_DIR/config/manager.local.json}"
LOCK_ACQUIRED="false"
DRY_RUN_FLAG=""
OFFLINE_FLAG=""
TASK_FAILURES=0
DOCTOR_STATUS="healthy"
DOCTOR_WARNING_OPTIONAL_TOOLS=0
DOCTOR_WARNING_UNMATERIALIZED=0
ORIGINAL_ARGS=("$@")
CMD_ARGS=()
cmd=""

# 如果本地覆盖文件存在，则读取并深度合并
if [ -f "$LOCAL_CONFIG" ]; then
    CONFIG_FILE="$(mktemp)"
    if ! python3 -c "
import json, sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping
from src.json_config import load_json_object_file
base = load_json_object_file('$BASE_CONFIG', strict=True, missing_ok=False)
try:
    local = load_json_object_file('$LOCAL_CONFIG', strict=True, missing_ok=False)
except ValueError as e:
    print(f'本地覆盖配置无效: {e}', file=sys.stderr)
    sys.exit(2)
def deep_update(dst, src):
    for k, v in src.items():
        if isinstance(v, dict) and isinstance(dst.get(k), dict):
            deep_update(dst[k], v)
        else:
            dst[k] = v
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
    # 没有本地覆盖时直接使用基线
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
[ -n "$cmd" ] || cmd="help"

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
            echo -e "\n${PENDING_GOLD}提示: 请运行 './manage.sh unlock' 手动强制解锁。${NC}"
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
        echo -e "\n${PENDING_GOLD}提示: 请运行 './manage.sh unlock' 手动强制解锁。${NC}"
        exit 1
    fi

    remove_lock_artifacts
    if mkdir "$LOCK_DIR" 2>/dev/null; then
        printf '%s\n' "$$" > "$LOCK_PID_FILE"
        printf '%s\n' "$$" > "$LOCK_FILE"
        LOCK_ACQUIRED="true"
        return 0
    fi

    echo -e "${ERROR_RED}错误: 无法获取运行锁，请稍后重试或执行 './manage.sh unlock'。${NC}"
    exit 1
}

# 原子锁检查
if [[ "$cmd" != "help" && "$cmd" != "-h" && "$cmd" != "--help" && "$cmd" != "-help" && "$cmd" != "detect" && "$cmd" != "doctor" && "$cmd" != "status" && "$cmd" != "backups:list" && "$cmd" != "unlock" && "$cmd" != "logs" && "$cmd" != "fetch-models" ]]; then
    acquire_lock
fi

function cleanup_on_exit() {
    if [ "$LOCK_ACQUIRED" = "true" ] && [ "$(read_lock_pid)" = "$$" ]; then
        remove_lock_artifacts
    fi
    [ -n "${CONFIG_FILE:-}" ] && [[ "$CONFIG_FILE" != "$BASE_CONFIG" ]] && rm -f "$CONFIG_FILE"

    local running_jobs=$(jobs -p)
    if [ -n "$running_jobs" ]; then
        kill $running_jobs 2>/dev/null || true
    fi
}
trap cleanup_on_exit EXIT

# 确保执行权限
chmod +x "$ROOT_DIR/agent-rule/scripts/"*.sh > /dev/null 2>&1 || true

# 系统探测
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

function update_latest_log_pointer() {
    mkdir -p "$(dirname "$TEMP_LOG")"
    : > "$TEMP_LOG"
    printf '%s\n' "$TEMP_LOG" > "$LATEST_LOG_POINTER"
    ln -sfn "$TEMP_LOG" "$LEGACY_LOG_PATH" 2>/dev/null || true
}

function resolve_latest_log_path() {
    if [ -f "$LATEST_LOG_POINTER" ]; then
        local candidate
        candidate="$(cat "$LATEST_LOG_POINTER" 2>/dev/null || true)"
        if [ -n "$candidate" ] && [ -f "$candidate" ]; then
            printf '%s\n' "$candidate"
            return 0
        fi
    fi
    if [ -f "$LEGACY_LOG_PATH" ] || [ -L "$LEGACY_LOG_PATH" ]; then
        printf '%s\n' "$LEGACY_LOG_PATH"
        return 0
    fi
    return 1
}

function detect_agents() {
    echo -e "${HDR_BLUE}${BOLD}1. 环境探测与健康自检 (Environment Health Check)${NC} ${BORDER_GRAY}[OS: $OS_TYPE]${NC}"
    $PYTHON_CMD -c "
import sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.json_config import ensure_mapping_value_types, get_typed_mapping_section
from src.manage_runtime_probe import (
    ProviderInventory,
    collect_active_mcp_target_agents,
    collect_active_rule_agents,
    collect_active_skill_target_agents,
    detect_all_providers,
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
        ensure_mapping_value_types(load_json('$ROOT_DIR/config/agents.json', strict=True, missing_ok=False), 'agents.json', (dict,), '对象')
    )
    tool_defs = get_typed_mapping_section(load_json('$ROOT_DIR/config/tools.json', strict=True, missing_ok=False), 'tools', 'tools.json', (dict,), '对象')
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
except ValueError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(1)
"
    echo ""
}

function validate_json_shape() {
    local file_path="$1"
    ROOT_DIR="$ROOT_DIR" FILE_PATH="$file_path" "$PYTHON_CMD" - <<'PY'
import os
import sys
from pathlib import Path

root = Path(os.environ["ROOT_DIR"])
path = Path(os.environ["FILE_PATH"])
sys.path.insert(0, str(root))

from src.json_config import ensure_mapping_value_types, get_typed_mapping_section, load_json_object_file

try:
    data = load_json_object_file(path, strict=True, missing_ok=False)
    name = path.name
    if name == "manager.json":
        get_typed_mapping_section(data, "agents", name, (dict,), "对象")
        get_typed_mapping_section(data, "tools", name, (dict,), "对象")
        get_typed_mapping_section(data, "skills", name, (dict,), "对象")
        get_typed_mapping_section(data, "mcp", name, (dict,), "对象")
    elif name == "tools.json":
        get_typed_mapping_section(data, "tools", name, (dict,), "对象")
    elif name == "skills.json":
        get_typed_mapping_section(data, "repositories", name, (dict,), "对象")
        get_typed_mapping_section(data, "agents", name, (str, dict), "字符串或对象")
    elif name == "mcp.json":
        get_typed_mapping_section(data, "servers", name, (dict,), "对象")
        get_typed_mapping_section(data, "agent_paths", name, (str, dict), "字符串或对象")
    elif name == "agents.json":
        ensure_mapping_value_types(data, name, (dict,), "对象")
except ValueError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(1)
PY
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
    local last_raw_msg=""
    local last_msg=""
    "$@" >> "$TEMP_LOG" 2>&1 &
    local pid=$!

    while kill -0 $pid 2>/dev/null; do
        local elapsed
        elapsed=$(( $(date +%s) - start_ts ))
        local current_display=""
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
            else
                current_display="$title... · 已运行 ${elapsed}s"
            fi
        else
            current_display="$title... · 已运行 ${elapsed}s"
        fi
        if [ "$current_display" != "$last_display" ]; then
            status_line "RUNNING" "$current_display"
            last_display="$current_display"
        fi
        sleep 0.3
    done
    if wait "$pid"; then
        status_line "DONE" "$success_msg"
        return 0
    else
        local exit_code=$?
        status_line "ERROR" "$err_msg"
        echo "FAILED: $title" >> "$TEMP_LOG"
        return "$exit_code"
    fi
}

function show_summary() {
    echo -e "${HDR_BLUE}${BOLD}3. 最终赋能详情汇总 (Deployment Architecture)${NC}"
    echo -e "${BORDER_GRAY}----------------------------------------------------------------${NC}"
    if [ ! -f "$TEMP_LOG" ]; then echo "   无运行记录。"; return; fi
    local clean_log=$(sed -E 's/^[0-9:-]+ [0-9:-]+ \[INFO\] //g; s/^INFO: //g' "$TEMP_LOG")
    echo -e "${BOLD}${HDR_BLUE}[ 生命周期与配置同步 ]${NC}"
    echo "$clean_log" | grep -E "状态对齐|更新 MCP" | sed 's/\[AGENT: \(.*\)\]/  ● \1/g' | sort -u || echo "   无变动。"
    echo -e "\n${BOLD}${HDR_BLUE}[ 规则与工具 ]${NC}"
    echo "$clean_log" | grep -E "同步完成|安装成功" | sed 's/\[AGENT: Rules\] //g; s/\[TOOL: \(.*\)\]/  ⚒ \1/g' | sort -u || echo "   无变动。"
    echo -e "${BORDER_GRAY}----------------------------------------------------------------${NC}"
    echo -e "${HDR_BLUE}${BOLD}4. 自动健康验证 (Post-Sync Verification)${NC}"
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
    run_task_with_progress "$title" "$success_msg" "$err_msg" timeout_run "$timeout_seconds" bash -c "cd \"$workdir\" && $command_str"
}

function begin_pipeline_execution() {
    detect_agents
    echo -e "${HDR_BLUE}${BOLD}2. 任务流水线 (Tasks)${NC}"
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

function maybe_init_secrets() {
    echo -e "${HDR_BLUE}${BOLD}1. 凭证仓库检查 (Secrets Check)${NC}"
    if [ -d "$ROOT_DIR/secrets/.git" ]; then
        if [ -n "$OFFLINE_FLAG" ]; then
            status_line "SKIP" "离线模式，跳过 secrets 拉取"
        elif [ -n "$DRY_RUN_FLAG" ]; then
            status_line "SKIP" "空转模式，跳过 secrets 拉取"
        else
            status_line "INFO" "拉取最新凭证..."
            git -C "$ROOT_DIR/secrets" pull --ff-only 2>&1 || \
                echo -e "${ERROR_RED}⚠️  secrets 拉取失败，将使用本地已有凭证${NC}"
        fi
    else
        if [ -n "$OFFLINE_FLAG" ]; then
            status_line "SKIP" "离线模式，跳过 secrets 检查"
        elif [ -n "$DRY_RUN_FLAG" ]; then
            status_line "SKIP" "空转模式，跳过 secrets 检查"
        else
            if [ -x "$ROOT_DIR/scripts/init-secrets.sh" ]; then
                bash "$ROOT_DIR/scripts/init-secrets.sh" || {
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
    run_task_in_dir "对齐 Agent 生命周期" "生命周期对齐完成" "生命周期失败" 600 "$ROOT_DIR/agent-lifecycle" "$PYTHON_CMD" "${args[@]}"
}

function run_tools_sync_task() {
    local args=(sync_tools.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/tools.json" --yes)
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$OFFLINE_FLAG" ]; then args+=("$OFFLINE_FLAG"); fi
    run_task_in_dir "同步全局工具" "工具链同步完成" "工具失败" 600 "$ROOT_DIR/agent-tool" "$PYTHON_CMD" "${args[@]}"
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
if not active_rule_agents:
    sys.exit(0)

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
    if rule_target_agent_id(parts[0]) in active_rule_agents:
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

    run_task_in_dir "$title" "MCP 注入完成" "MCP 失败" 600 "$ROOT_DIR/agent-mcp" "$PYTHON_CMD" "${args[@]}"
}

function run_terminal_sync_task() {
    local force_run="${1:-false}"
    if [ "$force_run" != "true" ]; then
        ROOT_DIR="$ROOT_DIR" CONFIG_FILE="$CONFIG_FILE" "$PYTHON_CMD" - <<'PY'
import os
import sys
from pathlib import Path

root = Path(os.environ["ROOT_DIR"])
sys.path.insert(0, str(root))

from src.json_config import load_json_object_file

cfg = load_json_object_file(os.environ["CONFIG_FILE"], strict=True, missing_ok=False)
terminal_cfg = cfg.get("terminal", {})
if terminal_cfg is None:
    terminal_cfg = {}
if not isinstance(terminal_cfg, dict):
    print("manager.json.terminal 必须是对象", file=sys.stderr)
    sys.exit(2)

sys.exit(0 if terminal_cfg.get("enabled", False) else 1)
PY
        local terminal_enabled_rc=$?
        if [ "$terminal_enabled_rc" -eq 2 ]; then
            return 1
        fi
        if [ "$terminal_enabled_rc" -ne 0 ]; then
            status_line "SKIP" "终端策略未启用，跳过终端配置同步"
            return 0
        fi
    fi

    local args=(sync_terminal.py)
    if [ -n "$DRY_RUN_FLAG" ]; then args+=("$DRY_RUN_FLAG"); fi
    if [ -n "$OFFLINE_FLAG" ]; then args+=("$OFFLINE_FLAG"); fi
    run_task_in_dir "同步终端配置" "终端配置同步完成" "终端配置失败" 120 "$ROOT_DIR/agent-terminal" "$PYTHON_CMD" "${args[@]}"
}

function run_sync_pipeline() {
    begin_workflow "🔄 Agent Manager 配置同步启动"
    begin_pipeline_execution
    run_pipeline_step run_rules_sync_task
    run_pipeline_step run_skills_link_task true
    run_pipeline_step run_mcp_sync_task false true
    run_pipeline_step run_terminal_sync_task
    finish_pipeline_execution
}

function run_update_pipeline() {
    local title="$1"
    local agent_extra_flags="$2"
    begin_workflow "$title"
    maybe_init_secrets
    begin_pipeline_execution
    run_pipeline_step run_agent_lifecycle_task "$agent_extra_flags"
    run_pipeline_step run_tools_sync_task
    run_pipeline_step run_rules_sync_task
    run_pipeline_step run_skills_link_task false
    run_pipeline_step run_mcp_sync_task true false
    run_pipeline_step run_terminal_sync_task
    finish_pipeline_execution
}

function run_self_update() {
    local repo_status
    if [ -n "$OFFLINE_FLAG" ]; then
        echo "离线模式，跳过 agent-manager 自更新。"
        return 0
    fi
    if [ -n "$DRY_RUN_FLAG" ]; then
        echo "预览模式：计划执行 agent-manager 仓库更新。"
        return 0
    fi
    if [ ! -d "$ROOT_DIR/.git" ]; then
        echo "当前目录不是 Git 仓库，跳过自更新。"
        return 0
    fi
    if ! repo_status=$(git -C "$ROOT_DIR" status --porcelain); then
        echo "无法检查仓库状态，跳过自更新。"
        return 1
    fi
    if [ -n "$repo_status" ]; then
        echo "检测到本地未提交更改，跳过自更新以保护工作区。"
        return 0
    fi
    git -C "$ROOT_DIR" pull --ff-only
}

function list_managed_target_paths() {
    local include_skills="${1:-false}"
    ROOT_DIR="$ROOT_DIR" INCLUDE_SKILLS="$include_skills" HOME="$HOME" "$PYTHON_CMD" - <<'PY'
import json
import os
import sys
from pathlib import Path

root = Path(os.environ["ROOT_DIR"])
include_skills = os.environ.get("INCLUDE_SKILLS") == "true"
targets = set()
sys.path.insert(0, str(root))

from src.json_config import get_typed_mapping_section, load_json_object_file

def add_target(value):
    if isinstance(value, dict):
        value = value.get("path")
    if not value:
        return
    expanded = str(value).replace("{{HOME}}", str(Path.home()))
    targets.add(str(Path(expanded).expanduser()))

try:
    mcp_cfg = load_json_object_file(root / "config" / "mcp.json", strict=True, missing_ok=False)
    mcp_agent_paths = get_typed_mapping_section(mcp_cfg, "agent_paths", "mcp.json", (str, dict), "字符串或对象")
except ValueError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(1)

for value in mcp_agent_paths.values():
    add_target(value)

if include_skills:
    try:
        skills_cfg = load_json_object_file(root / "config" / "skills.json", strict=True, missing_ok=False)
        skill_agents = get_typed_mapping_section(skills_cfg, "agents", "skills.json", (str, dict), "字符串或对象")
    except ValueError as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
    for value in skill_agents.values():
        add_target(value)

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
    add_target(parts[3])

for target in sorted(targets):
    print(target)
PY
}

function list_managed_backup_entries() {
    local targets_output
    if ! targets_output="$(list_managed_target_paths false)"; then
        return 1
    fi

    TARGETS_OUTPUT="$targets_output" "$PYTHON_CMD" - <<'PY'
import os
import sys

targets = [line for line in os.environ.get("TARGETS_OUTPUT", "").splitlines() if line.strip()]
entries = {}

for target in targets:
    target_dir = os.path.dirname(target)
    target_name = os.path.basename(target)
    try:
        names = os.listdir(target_dir)
    except OSError:
        continue

    for name in names:
        if name != f"{target_name}.bak" and not name.startswith(f"{target_name}.bak."):
            continue
        path = os.path.join(target_dir, name)
        try:
            mtime = os.path.getmtime(path)
        except OSError:
            continue
        entries[path] = max(entries.get(path, float("-inf")), mtime)

for path, _mtime in sorted(entries.items(), key=lambda item: (-item[1], item[0])):
    print(path)
PY
}

function backup_restore_target_path() {
    local backup_path="$1"
    if [[ "$backup_path" == *.bak.* ]]; then
        printf '%s\n' "${backup_path%.bak.*}"
        return
    fi
    if [[ "$backup_path" == *.bak ]]; then
        printf '%s\n' "${backup_path%.bak}"
        return
    fi
    return 1
}

function copy_backup_payload() {
    local backup_path="$1"
    local destination_path="$2"
    if [ -d "$backup_path" ] && [ ! -L "$backup_path" ]; then
        cp -pR "$backup_path" "$destination_path"
    else
        cp -pP "$backup_path" "$destination_path"
    fi
}

function restore_backup_entry() {
    local backup_path="$1"
    local target_path
    if ! target_path="$(backup_restore_target_path "$backup_path")"; then
        echo "无法推导备份目标: $backup_path"
        return 1
    fi

    local target_parent target_name staging_path rollback_path
    target_parent="$(dirname "$target_path")"
    target_name="$(basename "$target_path")"
    staging_path="$target_parent/.${target_name}.restore_tmp_${RUN_ID}"
    rollback_path="$target_parent/.${target_name}.restore_old_${RUN_ID}"

    mkdir -p "$target_parent" || return 1
    rm -rf "$staging_path" "$rollback_path"

    if ! copy_backup_payload "$backup_path" "$staging_path"; then
        rm -rf "$staging_path"
        echo "恢复失败：无法读取备份 $backup_path"
        return 1
    fi

    if [ -e "$target_path" ] || [ -L "$target_path" ]; then
        if ! mv "$target_path" "$rollback_path"; then
            rm -rf "$staging_path"
            echo "恢复失败：无法暂存现有目标 $target_path"
            return 1
        fi
    fi

    if mv "$staging_path" "$target_path"; then
        rm -rf "$rollback_path"
        return 0
    fi

    echo "恢复失败：无法写入目标 $target_path"
    rm -rf "$staging_path"
    if [ -e "$rollback_path" ] || [ -L "$rollback_path" ]; then
        mv "$rollback_path" "$target_path" 2>/dev/null || true
    fi
    return 1
}

function restore_backup_selection_input() {
    if [ -t 0 ] && [ -r /dev/tty ]; then
        printf '/dev/tty'
    else
        printf '/dev/stdin'
    fi
}

function is_dry_run_mode() {
    [ -n "$DRY_RUN_FLAG" ]
}

case "$cmd" in
    all|bootstrap)
        run_update_pipeline "🚀 Agent Manager 全量同步启动" ""
        ;;
    update)
        run_update_pipeline "🔄 Agent Manager 资源更新启动" "--no-install"
        ;;
    sync)
        run_sync_pipeline
        ;;
    plan)
        DRY_RUN_FLAG="--dry-run"
        run_update_pipeline "🧭 Agent Manager 变更预演启动" ""
        ;;
    self-update)
        run_self_update
        ;;
    offline)
        OFFLINE_FLAG="--offline"
        run_sync_pipeline
        ;;
    ui) (cd "$ROOT_DIR" && $PYTHON_CMD -m src.interactive) ;;
    doctor|status)
        if [ "${CMD_ARGS[0]:-}" = "--json" ]; then
            doctor_check json
        else
            doctor_check
        fi
        ;;
    detect) detect_agents ;;
    unlock)
        if is_dry_run_mode; then
            echo "预览模式：计划清理原子锁目录与遗留锁文件。"
        else
            remove_lock_artifacts
            echo "锁已解除。"
        fi ;;
    logs)
        if latest_log=$(resolve_latest_log_path); then
            if [ -t 1 ] && command -v less >/dev/null 2>&1; then
                less -R "$latest_log"
            else
                cat "$latest_log"
            fi
        else
            echo "暂无日志。"
        fi ;;
    clean-logs)
        if is_dry_run_mode; then
            echo "预览模式：计划清理 latest 指针与所有按次运行日志。"
        else
            rm -f "${LOG_PREFIX}"*.log "$LATEST_LOG_POINTER"
            echo "日志已清理"
        fi ;;
    backups:list)
        if ! backup_entries="$(list_managed_backup_entries)"; then
            exit 1
        fi
        if [ -n "$backup_entries" ]; then
            printf '%s\n' "$backup_entries" | head -n 20
        fi ;;
    backups:restore) 
        files=()
        if ! backup_entries="$(list_managed_backup_entries)"; then
            exit 1
        fi
        while IFS= read -r line; do
            files+=("$line")
        done <<< "$(printf '%s\n' "$backup_entries" | head -n 10)"
        if [ "${#files[@]}" -eq 0 ]; then echo "未发现备份。"; exit 0; fi
        if is_dry_run_mode; then
            echo "预览模式：以下备份可用于恢复（最多10个），实际恢复已跳过："
            printf '%s\n' "${files[@]}"
            exit 0
        fi
        selection_input="$(restore_backup_selection_input)"
        select f in "${files[@]}"; do
            [ -n "${f:-}" ] || { echo "无效选择。"; continue; }
            if restore_backup_entry "$f"; then
                echo "已恢复。"
            else
                exit 1
            fi
            break
        done < "$selection_input" ;;
    mcp:sync)
        (cd "$ROOT_DIR/agent-mcp" && $PYTHON_CMD sync_mcp.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/mcp.json" $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    mcp:prepare)
        (cd "$ROOT_DIR/agent-mcp" && $PYTHON_CMD sync_mcp.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/mcp.json" --prepare-runtime $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    rules:sync)
        if ! target_names="$(list_active_rule_targets)"; then
            exit 1
        fi
        if [ -z "$target_names" ]; then
            echo "当前策略未启用任何 Rules 目标。"
            exit 0
        fi
        rule_args=()
        while IFS= read -r target_name; do
            [ -n "$target_name" ] && rule_args+=(--target "$target_name")
        done <<< "$target_names"
        (cd "$ROOT_DIR/agent-rule" && ./scripts/sync-agent-rules.sh "${rule_args[@]}" $DRY_RUN_FLAG) ;;
    skills:link)
        (cd "$ROOT_DIR/agent-skill" && $PYTHON_CMD link_skills.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/skills.json" --yes $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    skills:upgrade)
        upgrade_args=(upgrade_skills.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/skills.json")
        if [ "${#CMD_ARGS[@]}" -gt 0 ]; then
            for repo_name in "${CMD_ARGS[@]}"; do
                upgrade_args+=(--repo "$repo_name")
            done
        fi
        if [ -n "$DRY_RUN_FLAG" ]; then upgrade_args+=("$DRY_RUN_FLAG"); fi
        if [ -n "$OFFLINE_FLAG" ]; then upgrade_args+=("$OFFLINE_FLAG"); fi
        (cd "$ROOT_DIR/agent-skill" && $PYTHON_CMD "${upgrade_args[@]}") ;;
    tools:sync)
        (cd "$ROOT_DIR/agent-tool" && $PYTHON_CMD sync_tools.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/tools.json" --yes $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    terminal:sync)
        run_terminal_sync_task true ;;
    fetch-models)
        args=()
        if [ "${#CMD_ARGS[@]}" -gt 0 ]; then
            args=("${CMD_ARGS[@]}")
        fi
        if [ "${#args[@]}" -eq 1 ]; then
            echo -e "${ERROR_RED}用法: ./manage.sh fetch-models [base_uri api_key]${NC}"
            exit 1
        fi
        if [ "${#args[@]}" -ge 2 ]; then
            $PYTHON_CMD "$ROOT_DIR/scripts/fetch_models.py" "${args[0]}" "${args[1]}"
        else
            $PYTHON_CMD "$ROOT_DIR/scripts/fetch_models.py"
        fi
        ;;
    cleanup)
        echo -e "${HDR_BLUE}正在执行全量环境大扫除...${NC}"
        if ! agents_paths="$(list_managed_target_paths true)"; then
            exit 1
        fi
        if is_dry_run_mode; then
            echo "预览模式：计划清理以下受管备份、临时文件与运行日志。"
        fi
        echo "$agents_paths" | while read -r p; do
            if [ -n "$p" ]; then
                d=$(dirname "$p")
                b=$(basename "$p")
                if [ -d "$d" ]; then
                    find "$d" -maxdepth 1 \( -name "${b}.bak" -o -name "${b}.bak.*" -o -name "${b}.tmp_*" -o -name ".${b}.restore_tmp_*" -o -name ".${b}.restore_old_*" \) -print0 2>/dev/null | while IFS= read -r -d '' candidate; do
                        if is_dry_run_mode; then
                            echo "  - $candidate"
                        else
                            rm -rf "$candidate"
                        fi
                    done
                fi
            fi
        done
        if is_dry_run_mode; then
            for log_candidate in "${LOG_PREFIX}"*.log "$LATEST_LOG_POINTER"; do
                if [ -e "$log_candidate" ] || [ -L "$log_candidate" ]; then
                    echo "  - $log_candidate"
                fi
            done
            echo -e "${SUCCESS_GREEN}全量清理预演完成。${NC}"
        else
            rm -f "${LOG_PREFIX}"*.log "$LATEST_LOG_POINTER"
            echo -e "${SUCCESS_GREEN}全量清理完成。${NC}"
        fi ;;
    help|-h|--help|-help)
        echo -e "${HDR_BLUE}${BOLD}Agent Manager 帮助手册 (Platinum Edition)${NC}\n"
        echo -e "直接执行 ${BOLD}./manage.sh${NC} 时默认显示本帮助，不会触发同步。\n"
        echo -e "${BOLD}${HDR_BLUE}=== 全局参数 (Flags) ===${NC}"
        echo -e "  ${PENDING_GOLD}--dry-run${NC}       空转模式，仅输出计划动作并跳过会修改本地或远端状态的副作用"
        echo -e "  ${PENDING_GOLD}--offline${NC}       离线模式，禁止远程拉取/安装；仅使用本地已存在仓库与配置\n"
        echo -e "${BOLD}${HDR_BLUE}=== 日常入口 (Main Flows) ===${NC}"
        echo -e "  ${SUCCESS_GREEN}sync${NC}            仅同步本地配置到 rules / skills / MCP，不联网、不安装"
        echo -e "  ${SUCCESS_GREEN}update${NC}          更新远端资源并收敛本地环境，但不安装缺失 Agent"
        echo -e "  ${SUCCESS_GREEN}bootstrap${NC}       执行全量安装、更新与同步，适合新机器初始化"
        echo -e "  ${SUCCESS_GREEN}all${NC}             \`bootstrap\` 的兼容别名"
        echo -e "  ${SUCCESS_GREEN}plan${NC}            预演一次完整 \`bootstrap\`（dry-run）"
        echo -e "  ${SUCCESS_GREEN}self-update${NC}     仅更新 agent-manager 仓库自身逻辑"
        echo -e "  ${SUCCESS_GREEN}ui${NC}              启动基于 Terminal 的交互式的可视化管理终端界面"
        echo -e "  ${SUCCESS_GREEN}doctor / status${NC} 进行深度的资源校验、配置自检与环境排错诊断"
        echo -e "  ${SUCCESS_GREEN}detect${NC}          仅执行各 Agent 开发工具的环境探测与支持情况检测"
        echo -e "  ${SUCCESS_GREEN}fetch-models${NC}    探测并获取模型列表。用法: ./manage.sh fetch-models <url> <key>\n"
        echo -e "${BOLD}${HDR_BLUE}=== 运维排错 (Maintenance) ===${NC}"
        echo -e "  ${VAL_PURPLE}cleanup${NC}         执行全量环境大扫除，深度清理临时文件、无用快照缓存及历史日志"
        echo -e "  ${VAL_PURPLE}unlock${NC}          强制清除原子锁目录与遗留锁文件"
        echo -e "  ${VAL_PURPLE}backups:list${NC}    查找并列出近期系统中可用的容灾备份配置快照，最多20个"
        echo -e "  ${VAL_PURPLE}backups:restore${NC} 交互式快照选择，用于从特定备份快照中恢复配置"
        echo -e "  ${VAL_PURPLE}logs${NC}            查看最近一次执行的完整全链路同步日志（latest 指针）"
        echo -e "  ${VAL_PURPLE}clean-logs${NC}      清空 latest 指针与所有按次留存的运行日志文件\n"
        echo -e "${BOLD}${HDR_BLUE}=== 分步原子控制 (Atomic syncs) ===${NC}"
        echo -e "  ${BORDER_GRAY}mcp:sync${NC}        仅执行 MCP 配置注入（默认不 clone / build / pull secrets）"
        echo -e "  ${BORDER_GRAY}mcp:prepare${NC}     显式准备 MCP 运行时（clone/pull repo、同步 secrets、setup/build）"
        echo -e "  ${BORDER_GRAY}rules:sync${NC}      独立执行规则与系统级 prompt 指令向各个 Agent 的拷贝或软链接分发"
        echo -e "  ${BORDER_GRAY}skills:link${NC}     独立执行全局或系统级专业技能代码集到 Agent 沙盒空间的映射配置与软链接"
        echo -e "  ${BORDER_GRAY}skills:upgrade${NC}  显式解析并回写 skill 仓库的最新可审计版本（仅 release/tag，失败即停止）"
        echo -e "  ${BORDER_GRAY}tools:sync${NC}      独立同步或安装全局统一增强工具链条"
        echo -e "  ${BORDER_GRAY}terminal:sync${NC}   独立同步终端工具（iTerm2/tmux）配置模板到家目录"
        echo "" ;;
    *) echo -e "${ERROR_RED}错误: 未知命令 '$cmd'${NC}"; exit 1 ;;
esac
