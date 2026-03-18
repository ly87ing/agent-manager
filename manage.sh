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
BASE_CONFIG="$ROOT_DIR/config/manager.json"
# 本地覆盖（不参与 Git）
LOCAL_CONFIG="$ROOT_DIR/config/manager.local.json"
LOCK_ACQUIRED="false"
DRY_RUN_FLAG=""
OFFLINE_FLAG=""
TASK_FAILURES=0
ORIGINAL_ARGS=("$@")
cmd=""

# 如果本地覆盖文件存在，则读取并深度合并
if [ -f "$LOCAL_CONFIG" ]; then
    CONFIG_FILE="$(mktemp)"
    if ! python3 -c "
import json, sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping
with open('$BASE_CONFIG', 'r', encoding='utf-8') as f:
    base = json.load(f)
try:
    with open('$LOCAL_CONFIG', 'r', encoding='utf-8') as f:
        local = json.load(f)
except Exception as e:
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
            fi
            ;;
    esac
    shift
done
[ -n "$cmd" ] || cmd="all"

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
if [[ "$cmd" != "help" && "$cmd" != "-h" && "$cmd" != "--help" && "$cmd" != "-help" && "$cmd" != "detect" && "$cmd" != "doctor" && "$cmd" != "status" && "$cmd" != "backups:list" && "$cmd" != "backups:restore" && "$cmd" != "unlock" && "$cmd" != "logs" ]]; then
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
import os, shutil, sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.manage_runtime_probe import ProviderInventory, detect_all_providers, load_json

try:
    inventory = ProviderInventory()
    cfg = load_json('$CONFIG_FILE', strict=True, missing_ok=False)
    cfg['agents'] = canonicalize_agent_mapping(cfg.get('agents', {}))
    def_a = canonicalize_agent_mapping(load_json('$ROOT_DIR/config/agents.json', strict=True, missing_ok=False))
    for k, v in cfg.get('agents', {}).items():
        if not v.get('enabled', True): continue
        d = def_a.get(k, {})
        label = get_agent_display_name(k, d)
        all_act = detect_all_providers(d.get('bin',''), d, inventory=inventory)
        stat = '${SUCCESS_GREEN}✓${NC}' if all_act else '${PENDING_GOLD}○${NC}'
        print(f'   {stat} {label:20} (安装源:{\"/\".join(all_act) if all_act else \"未发现\"})')
    for k, v in cfg.get('tools', {}).items():
        if v.get('enabled', True):
            act = shutil.which(k) is not None or os.path.exists(f'$ROOT_DIR/agent-tool/{k}')
            stat = '${SUCCESS_GREEN}⚒${NC}' if act else '${PENDING_GOLD}⚠${NC}'
            print(f'   {stat} {k:20} (工具就绪:{\"是\" if act else \"否\"})')
except ValueError as exc:
    print(str(exc), file=sys.stderr)
    sys.exit(1)
"
    echo ""
}

function doctor_check() {
    local silent="${1:-false}"
    [ "$silent" == "false" ] && echo -e "${HDR_BLUE}${BOLD}Agent Manager 深度诊断 (Doctor Check)${NC}\n"
    local errors=0
    
    if [ "$silent" == "false" ] && [ -d "$ROOT_DIR/.git" ]; then
        echo -e "${BOLD}[ 管理系统版本 ]${NC}  ${VAL_PURPLE}$(git -C "$ROOT_DIR" rev-parse --short HEAD)${NC} ($(git -C "$ROOT_DIR" log -1 --format=%cd --date=short))\n"
    fi

    # 运行环境
    if [ "$silent" == "false" ]; then
        echo -e "${BOLD}[ 运行环境 ]${NC}"
        for c in "$PYTHON_CMD" "node" "npm" "git" "make"; do
            command -v $c >/dev/null 2>&1 && echo -e "  ${SUCCESS_GREEN}✓${NC} $c: $($c --version | head -n1)" || { echo -e "  ${ERROR_RED}✗${NC} $c: 缺失"; errors=$((errors+1)); }
        done
        echo -e "\n${BOLD}[ 存储空间 ]${NC}"
        echo -e "  ${SUCCESS_GREEN}✓${NC} 磁盘可用: $(df -h . | tail -n1 | awk '{print $4}')\n"
    fi

    # 配置审计
    [ "$silent" == "false" ] && echo -e "${BOLD}[ 配置与安全 ]${NC}"
    for f in "$ROOT_DIR"/config/*.json; do
        if $PYTHON_CMD -m json.tool "$f" >/dev/null 2>&1; then
            [ "$silent" == "false" ] && echo -ne "  ${SUCCESS_GREEN}✓${NC} $(basename "$f"): 格式正确"
            local secrets=$($PYTHON_CMD -c "import re; c=open('$f').read(); f=re.findall(r'(sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]{20,}|AIza[a-zA-Z0-9_\\\\-]{20,})', c); print(' '.join(f))")
            if [ -n "$secrets" ]; then echo -e " ${ERROR_RED}(⚠ 包含明文密钥风险!)${NC}"; errors=$((errors+1)); else [ "$silent" == "false" ] && echo ""; fi
        else echo -e "  ${ERROR_RED}✗${NC} $(basename "$f"): 语法错误"; errors=$((errors+1)); fi
    done

    if [ $errors -gt 0 ]; then
        [ "$silent" == "false" ] && echo -e "${ERROR_RED}${BOLD}❌ 认证失败：发现 $errors 个配置问题。${NC}"
        return 1
    fi

    # 一致性审计
    [ "$silent" == "false" ] && echo -e "\n${BOLD}[ 路径、内容与链接一致性审计 ]${NC}"
    local audit_out=$($PYTHON_CMD -c "
import os, hashlib, re, sys
sys.path.insert(0, '$ROOT_DIR')
from src.agent_ids import canonicalize_agent_mapping, get_agent_display_name
from src.manage_runtime_probe import ProviderInventory, detect_all_providers, load_json

def get_md5(p):
    try: return hashlib.md5(open(p, 'rb').read()).hexdigest()
    except: return None

def build_tree_signature(root):
    sig = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames.sort()
        filenames.sort()
        rel_dir = os.path.relpath(dirpath, root)
        rel_dir = '' if rel_dir == '.' else rel_dir
        for dirname in dirnames:
            rel_path = os.path.join(rel_dir, dirname).replace(os.sep, '/')
            sig[f'{rel_path}/'] = 'dir'
        for filename in filenames:
            rel_path = os.path.join(rel_dir, filename).replace(os.sep, '/')
            sig[rel_path] = get_md5(os.path.join(dirpath, filename))
    return sig

def rules_match(src, dst):
    if os.path.isdir(src) or os.path.isdir(dst):
        if not (os.path.isdir(src) and os.path.isdir(dst)):
            return False
        return build_tree_signature(src) == build_tree_signature(dst)
    return get_md5(src) == get_md5(dst)
inventory = ProviderInventory()
errs = 0
cfg = load_json('$CONFIG_FILE')
cfg['agents'] = canonicalize_agent_mapping(cfg.get('agents', {}))
def_a = canonicalize_agent_mapping(load_json('$ROOT_DIR/config/agents.json'))
# 1. Agent Policy
print('${BOLD}--- Agent 状态与策略一致性 ---${NC}')
for k, v in cfg.get('agents', {}).items():
    d = def_a.get(k, {})
    label = get_agent_display_name(k, d)
    all_act = detect_all_providers(d.get('bin',''), d, inventory=inventory)
    is_act = len(all_act) > 0
    des = v.get('enabled', True)
    pref = v.get('preferred_provider') or d.get('preferred_provider', 'npm')
    if is_act == des:
        detail = f'({all_act[0] if is_act else \"未安装\"})'
        # 允许策略中的 brew 匹配实际的 brew 或 brew-cask
        act_base = all_act[0].split('-')[0] if is_act else \"\"
        if is_act and act_base != pref.split('-')[0]: detail += f' ${PENDING_GOLD}(偏离偏好: {pref})${NC}'
        print(f'  ${SUCCESS_GREEN}✓${NC} {label:20}: {detail}')
    else: 
        print(f'  ${ERROR_RED}✗${NC} {label:20}: 漂移 (实际:{\"已装\" if is_act else \"未装\"})')
        errs += 1
for k, d in def_a.items():
    if k not in cfg.get('agents', {}):
        print(f'  ${PENDING_GOLD}⚠${NC} {get_agent_display_name(k, d):20}: 未声明策略（已忽略）')
# 2. MCP
print('\n${BOLD}--- MCP 注入审计 ---${NC}')
ap = canonicalize_agent_mapping(load_json('$ROOT_DIR/config/mcp.json').get('agent_paths', {}))
for k, v in ap.items():
    p = os.path.expanduser(v if isinstance(v, str) else v.get('path', ''))
    if os.path.exists(p):
        cnt = len(re.findall(r'_MANAGED_BY', open(p).read()))
        print(f'  ${SUCCESS_GREEN}✓${NC} {get_agent_display_name(k)}: 已注入 {cnt} 个托管服务')
    else:
        print(f'  ${ERROR_RED}✗${NC} {get_agent_display_name(k)}: 缺失')
        errs += 1
# 3. Skills
print('\n${BOLD}--- Skills 链接审计 ---${NC}')
s_cfg = load_json('$ROOT_DIR/config/skills.json')
for a_name, a_path in canonicalize_agent_mapping(s_cfg.get('agents', {})).items():
    p = os.path.expanduser(a_path)
    if os.path.exists(p):
        links = [f for f in os.listdir(p) if os.path.islink(os.path.join(p, f))]
        print(f'  ● {get_agent_display_name(a_name):15}: {len(links)} 个物理链接')
# 4. Rules
print('\n${BOLD}--- Rules 内容/结构审计 ---${NC}')
rm = '$ROOT_DIR/config/rules-targets.list'
if os.path.exists(rm):
    for line in open(rm):
        stripped = line.strip()
        if not stripped or stripped.startswith('#'): continue
        p = [part.strip() for part in stripped.split('|')]
        if len(p) < 4:
            print(f'  ${ERROR_RED}✗${NC} {stripped}: manifest 非法')
            errs += 1
            continue
        src = os.path.join('$ROOT_DIR/agent-rule', p[2])
        dst = os.path.expanduser(p[3].replace('{{HOME}}', os.path.expanduser('~')))
        if os.path.exists(dst):
            if not rules_match(src, dst): print(f'  ${PENDING_GOLD}⚠${NC} {p[0]}: 漂移'); errs += 1
            else: print(f'  ${SUCCESS_GREEN}✓${NC} {p[0]}: 一致')
        else: print(f'  ${ERROR_RED}✗${NC} {p[0]}: 缺失'); errs += 1
print(f'__ERRORS__:{errs}')
")
    [ "$silent" == "false" ] && echo "$audit_out" | grep -v "__ERRORS__"
    errors=$((errors + $(echo "$audit_out" | grep "__ERRORS__" | cut -d: -f2)))

    # 容灾快照
    if [ "$silent" == "false" ]; then
        echo -e "\n${BOLD}[ 容灾快照统计 ]${NC}"
        local baks=$(find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) 2>/dev/null | wc -l | tr -d ' ')
        echo -e "  ${SUCCESS_GREEN}✓${NC} 有效历史快照: $baks 个\n"
    fi

    if [ $errors -eq 0 ]; then
        [ "$silent" == "false" ] && echo -e "${SUCCESS_GREEN}${BOLD}✨ 认证通过：环境状态完美。${NC}"
        return 0
    else
        [ "$silent" == "false" ] && echo -e "${ERROR_RED}${BOLD}❌ 认证失败：发现 $errors 个一致性问题。${NC}"
        return 1
    fi
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
    doctor_check silent && echo -e "   ${SUCCESS_GREEN}${BOLD}CERTIFIED HEALTHY${NC}" || echo -e "   ${ERROR_RED}${BOLD}HEALTH WARNING${NC}"
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
    if [ -d "$ROOT_DIR/secrets/.git" ]; then
        return 0
    fi
    echo -e "${HDR_BLUE}${BOLD}1. 凭证仓库检查 (Secrets Check)${NC}"
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

function run_rules_sync_task() {
    local args=(./scripts/sync-agent-rules.sh)
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

function run_sync_pipeline() {
    begin_workflow "🔄 Agent Manager 配置同步启动"
    begin_pipeline_execution
    run_pipeline_step run_rules_sync_task
    run_pipeline_step run_skills_link_task true
    run_pipeline_step run_mcp_sync_task false true
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
    git -C "$ROOT_DIR" pull
}

case "$cmd" in
    all|""|bootstrap)
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
    doctor|status) doctor_check ;;
    detect) detect_agents ;;
    unlock) remove_lock_artifacts; echo "锁已解除。" ;;
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
    clean-logs) rm -f "${LOG_PREFIX}"*.log "$LATEST_LOG_POINTER"; echo "日志已清理" ;;
    backups:list) find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) 2>/dev/null | sort -r | head -n 20 || true ;;
    backups:restore) 
        files=$(find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) 2>/dev/null | sort -r | head -n 10 || true)
        if [ -z "$files" ]; then echo "未发现备份。"; exit 0; fi
        select f in $files; do cp -i "$f" "${f%.bak.*}"; echo "已恢复。"; break; done < /dev/tty ;;
    mcp:sync)
        (cd "$ROOT_DIR/agent-mcp" && $PYTHON_CMD sync_mcp.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/mcp.json" $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    mcp:prepare)
        (cd "$ROOT_DIR/agent-mcp" && $PYTHON_CMD sync_mcp.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/mcp.json" --prepare-runtime $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    rules:sync)
        (cd "$ROOT_DIR/agent-rule" && ./scripts/sync-agent-rules.sh $DRY_RUN_FLAG) ;;
    skills:link)
        (cd "$ROOT_DIR/agent-skill" && $PYTHON_CMD link_skills.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/skills.json" --yes $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    tools:sync)
        (cd "$ROOT_DIR/agent-tool" && $PYTHON_CMD sync_tools.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/tools.json" --yes $DRY_RUN_FLAG $OFFLINE_FLAG) ;;
    cleanup)
        echo -e "${HDR_BLUE}正在执行全量环境大扫除...${NC}"
        # (cd "$ROOT_DIR/agent-rule" && ./scripts/clean-agent-rule-backups.sh --yes || true)
        agents_paths=$($PYTHON_CMD -c "
import json, os
def expand(v): 
    return os.path.expanduser(v if isinstance(v,str) else v.get('path',''))

targets = []
try:
    m=json.load(open('$ROOT_DIR/config/mcp.json')).get('agent_paths',{}).values()
    s=json.load(open('$ROOT_DIR/config/skills.json')).get('agents',{}).values()
    for p in list(m)+list(s):
        if p: targets.append(expand(p))

    # 加入 rule 的备份路径
    rm = '$ROOT_DIR/config/rules-targets.list'
    if os.path.exists(rm):
        for line in open(rm):
            if not line.strip() or line.startswith('#'): continue
            dst = line.strip().split('|')[3].replace('{{HOME}}', '~')
            targets.append(os.path.expanduser(dst))
except Exception:
    pass

for t in set(targets): print(t)
")
        echo "$agents_paths" | while read -r p; do
            if [ -n "$p" ]; then
                d=$(dirname "$p")
                b=$(basename "$p")
                if [ -d "$d" ]; then
                    find "$d" -maxdepth 1 \( -name "${b}.bak.*" -o -name "${b}.tmp_*" -o -name "*.tmp_*" \) -type f -delete 2>/dev/null || true
                fi
            fi
        done
        rm -f "${LOG_PREFIX}"*.log "$LATEST_LOG_POINTER"
        echo -e "${SUCCESS_GREEN}全量清理完成。${NC}" ;;
    help|-h|--help|-help)
        echo -e "${HDR_BLUE}${BOLD}Agent Manager 帮助手册 (Platinum Edition)${NC}\n"
        echo -e "${BOLD}${HDR_BLUE}=== 全局参数 (Flags) ===${NC}"
        echo -e "  ${PENDING_GOLD}--dry-run${NC}       空转模式，仅输出计划动作并跳过 Git / secrets 等远端副作用"
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
        echo -e "  ${SUCCESS_GREEN}detect${NC}          仅执行各 Agent 开发工具的环境探测与支持情况检测\n"
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
        echo -e "  ${BORDER_GRAY}tools:sync${NC}      独立同步或安装全局统一增强工具链条"
        echo "" ;;
    *) echo -e "${ERROR_RED}错误: 未知命令 '$cmd'${NC}"; exit 1 ;;
esac
