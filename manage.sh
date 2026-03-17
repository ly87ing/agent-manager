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
TEMP_LOG="/tmp/agent_manager_sync_${USER_NAME}.log"
LOCK_FILE="/tmp/agent_manager_${USER_NAME}.lock"
# 基线策略（仓库提供）
BASE_CONFIG="$ROOT_DIR/config/manager.json"
# 本地覆盖（不参与 Git）
LOCAL_CONFIG="$ROOT_DIR/config/manager.local.json"
LOCK_ACQUIRED="false"

# 如果本地覆盖文件存在，则读取并深度合并
if [ -f "$LOCAL_CONFIG" ]; then
    CONFIG_FILE="$(mktemp)"
    python3 -c "
import json, sys
with open('$BASE_CONFIG', 'r', encoding='utf-8') as f:
    base = json.load(f)
try:
    with open('$LOCAL_CONFIG', 'r', encoding='utf-8') as f:
        local = json.load(f)
except Exception as e:
    local = {}
def deep_update(dst, src):
    for k, v in src.items():
        if isinstance(v, dict) and isinstance(dst.get(k), dict):
            deep_update(dst[k], v)
        else:
            dst[k] = v
deep_update(base, local)
with open('$CONFIG_FILE', 'w', encoding='utf-8') as f:
    json.dump(base, f, indent=2, ensure_ascii=False)
"
else
    # 没有本地覆盖时直接使用基线
    CONFIG_FILE="$BASE_CONFIG"
fi

# 原子锁检查
if [[ "${1:-}" != "help" && "${1:-}" != "-h" && "${1:-}" != "--help" && "${1:-}" != "-help" && "${1:-}" != "detect" && "${1:-}" != "doctor" && "${1:-}" != "backups:list" && "${1:-}" != "backups:restore" && "${1:-}" != "unlock" && "${1:-}" != "logs" ]]; then
    if [ -f "$LOCK_FILE" ]; then
        old_pid=$(cat "$LOCK_FILE")
        if [[ "$old_pid" != "$$" ]] && ps -p "$old_pid" > /dev/null 2>&1; then
            echo -e "${ERROR_RED}错误: 另一个管理任务 (PID: $old_pid) 正在运行。${NC}"
            ps -fp "$old_pid" | sed 's/^/  /' || true
            echo -e "\n${PENDING_GOLD}提示: 请运行 './manage.sh unlock' 手动强制解锁。${NC}"
            exit 1
        fi
    fi
    echo $$ > "$LOCK_FILE"
    LOCK_ACQUIRED="true"
fi

function cleanup_on_exit() {
    if [ "$LOCK_ACQUIRED" = "true" ] && [ -f "$LOCK_FILE" ] && [ "$(cat "$LOCK_FILE" 2>/dev/null)" = "$$" ]; then
        rm -f "$LOCK_FILE"
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

function detect_agents() {
    echo -e "${HDR_BLUE}${BOLD}1. 环境探测与健康自检 (Environment Health Check)${NC} ${BORDER_GRAY}[OS: $OS_TYPE]${NC}"
    $PYTHON_CMD -c "
import json, os, shutil, subprocess, re
def load(p):
    try: return json.load(open(p))
    except: return {}

def run_cmd(cmd):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=10)
    except Exception:
        class Dummy:
            returncode = 1
            stdout = ''
        return Dummy()

# 预先一次性加载所有包列表（关键优化：避免逐个包执行 brew list）
_brew_formulae = set()
_brew_casks = set()
_gh_exts = ''

if shutil.which('brew'):
    res = run_cmd('brew list --formula -1')
    if res.returncode == 0 and res.stdout.strip():
        _brew_formulae = set(res.stdout.strip().split('\n'))
    res = run_cmd('brew list --cask -1')
    if res.returncode == 0 and res.stdout.strip():
        _brew_casks = set(res.stdout.strip().split('\n'))

if shutil.which('gh'):
    res = run_cmd('gh extension list')
    if res.returncode == 0:
        _gh_exts = res.stdout

def get_pkg_name(cmd):
    if not cmd: return None
    m = re.search(r'(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)', cmd)
    return m.group(1) if m else None

def detect_all_providers(bin_name, config):
    found = []
    p_path = shutil.which(bin_name)
    is_npm = p_path and ('/node/' in p_path or '/npm/' in p_path or p_path.endswith('node') or p_path.endswith('npm'))
    
    brew_pkg = get_pkg_name(config.get('providers', {}).get('brew', {}).get('install', ''))
    if brew_pkg:
        if brew_pkg in _brew_formulae: found.append('brew')
        elif brew_pkg in _brew_casks: found.append('brew-cask')
    
    if is_npm: found.append('npm')
    
    ext_name = config.get('ext_name')
    if ext_name and any(line.strip().startswith(ext_name) for line in _gh_exts.splitlines()):
        found.append('gh')
    
    app_path = config.get('app_path')
    if (app_path and os.path.exists(app_path)) or p_path:
        if not found: found.append('system')
    return found

cfg = load('$CONFIG_FILE')
def_a = load('$ROOT_DIR/config/agents.json')
for k, v in cfg.get('agents', {}).items():
    if not v.get('enabled', True): continue
    d = def_a.get(k, {})
    all_act = detect_all_providers(d.get('bin',''), d)
    stat = '${SUCCESS_GREEN}✓${NC}' if all_act else '${PENDING_GOLD}○${NC}'
    print(f'   {stat} {k:20} (安装源:{\"/\".join(all_act) if all_act else \"未发现\"})')
for k, v in cfg.get('tools', {}).items():
    if v.get('enabled', True):
        act = shutil.which(k) is not None or os.path.exists(f'$ROOT_DIR/agent-tool/{k}')
        stat = '${SUCCESS_GREEN}⚒${NC}' if act else '${PENDING_GOLD}⚠${NC}'
        print(f'   {stat} {k:20} (工具就绪:{\"是\" if act else \"否\"})')
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

    # 一致性审计
    [ "$silent" == "false" ] && echo -e "\n${BOLD}[ 路径、内容与链接一致性审计 ]${NC}"
    local audit_out=$($PYTHON_CMD -c "
import json, os, hashlib, shutil, re, subprocess
def get_md5(p):
    try: return hashlib.md5(open(p, 'rb').read()).hexdigest()
    except: return None

def run_cmd(cmd):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=10)
    except Exception:
        class Dummy:
            returncode = 1
            stdout = ''
        return Dummy()

# 预先一次性加载所有包列表（关键优化）
_brew_formulae = set()
_brew_casks = set()
_gh_exts = ''

if shutil.which('brew'):
    res = run_cmd('brew list --formula -1')
    if res.returncode == 0 and res.stdout.strip():
        _brew_formulae = set(res.stdout.strip().split('\n'))
    res = run_cmd('brew list --cask -1')
    if res.returncode == 0 and res.stdout.strip():
        _brew_casks = set(res.stdout.strip().split('\n'))

if shutil.which('gh'):
    res = run_cmd('gh extension list')
    if res.returncode == 0:
        _gh_exts = res.stdout

def get_pkg_name(cmd):
    if not cmd: return None
    m = re.search(r'(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)', cmd)
    return m.group(1) if m else None

def detect_all_providers(bin_name, config):
    found = []
    p_path = shutil.which(bin_name)
    is_npm = p_path and ('/node/' in p_path or '/npm/' in p_path or p_path.endswith('node') or p_path.endswith('npm'))

    brew_pkg = get_pkg_name(config.get('providers', {}).get('brew', {}).get('install', ''))
    if brew_pkg:
        if brew_pkg in _brew_formulae: found.append('brew')
        elif brew_pkg in _brew_casks: found.append('brew-cask')
    
    if is_npm: found.append('npm')
    ext_name = config.get('ext_name')
    if ext_name and any(line.strip().startswith(ext_name) for line in _gh_exts.splitlines()):
        found.append('gh')
    
    app_path = config.get('app_path')
    if (app_path and os.path.exists(app_path)) or p_path:
        if not found: found.append('system')
    return found

errs = 0
cfg = json.load(open('$CONFIG_FILE'))
def_a = json.load(open('$ROOT_DIR/config/agents.json'))
# 1. Agent Policy
print('${BOLD}--- Agent 状态与策略一致性 ---${NC}')
for k, v in cfg.get('agents', {}).items():
    d = def_a.get(k, {})
    all_act = detect_all_providers(d.get('bin',''), d)
    is_act = len(all_act) > 0
    des = v.get('enabled', True)
    pref = v.get('preferred_provider') or d.get('preferred_provider', 'npm')
    if is_act == des:
        detail = f'({all_act[0] if is_act else \"未安装\"})'
        # 允许策略中的 brew 匹配实际的 brew 或 brew-cask
        act_base = all_act[0].split('-')[0] if is_act else \"\"
        if is_act and act_base != pref.split('-')[0]: detail += f' ${PENDING_GOLD}(偏离偏好: {pref})${NC}'
        print(f'  ${SUCCESS_GREEN}✓${NC} {k:20}: {detail}')
    else: 
        print(f'  ${ERROR_RED}✗${NC} {k:20}: 漂移 (实际:{\"已装\" if is_act else \"未装\"})')
        errs += 1
# 2. MCP
print('\n${BOLD}--- MCP 注入审计 ---${NC}')
ap = json.load(open('$ROOT_DIR/config/mcp.json')).get('agent_paths', {})
for k, v in ap.items():
    p = os.path.expanduser(v if isinstance(v, str) else v.get('path', ''))
    if os.path.exists(p):
        cnt = len(re.findall(r'_MANAGED_BY', open(p).read()))
        print(f'  ${SUCCESS_GREEN}✓${NC} {k}: 已注入 {cnt} 个托管服务')
    else: print(f'  ${ERROR_RED}✗${NC} {k}: 缺失'); errs += 1
# 3. Skills
print('\n${BOLD}--- Skills 链接审计 ---${NC}')
s_cfg = json.load(open('$ROOT_DIR/config/skills.json'))
for a_name, a_path in s_cfg.get('agents', {}).items():
    p = os.path.expanduser(a_path)
    if os.path.exists(p):
        links = [f for f in os.listdir(p) if os.path.islink(os.path.join(p, f))]
        print(f'  ● {a_name:15}: {len(links)} 个物理链接')
# 4. Rules
print('\n${BOLD}--- Rules 内容 MD5 审计 ---${NC}')
rm = '$ROOT_DIR/config/rules-targets.list'
if os.path.exists(rm):
    for line in open(rm):
        if not line.strip() or line.startswith('#'): continue
        p = line.strip().split('|')
        src, dst = os.path.join('$ROOT_DIR/agent-rule', p[2]), os.path.expanduser(p[3].replace('{{HOME}}', '~'))
        if os.path.exists(dst):
            if get_md5(src) != get_md5(dst): print(f'  ${PENDING_GOLD}⚠${NC} {p[0]}: 漂移'); errs += 1
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
    "$@" >> "$TEMP_LOG" 2>&1 &
    local pid=$!

    while kill -0 $pid 2>/dev/null; do
        if [ -s "$TEMP_LOG" ]; then
            local last_msg=$(tail -n 1 "$TEMP_LOG" 2>/dev/null | sed -E 's/^[0-9-]+ [0-9:]+ \[(INFO|WARNING|ERROR)\] //g; s/INFO: //g; s/^[[:space:]]*//; s/\r//g' | awk '{print substr($0,1,55)}')
            if [ -n "$last_msg" ]; then
                status_line "RUNNING" "$title: $last_msg"
            fi
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

DRY_RUN_FLAG=""
OFFLINE_FLAG=""
TASK_FAILURES=0
case "${1:-}" in
    --dry-run) cmd="all"; DRY_RUN_FLAG="--dry-run"; shift ;;
    --offline) cmd="all"; OFFLINE_FLAG="--offline"; shift ;;
    *) if [ $# -eq 0 ]; then cmd="all"; else cmd="$1"; shift; fi
       [[ "$*" == *"--dry-run"* ]] && DRY_RUN_FLAG="--dry-run"
       [[ "$*" == *"--offline"* ]] && OFFLINE_FLAG="--offline" ;;
esac

case "$cmd" in
    all|"")
        clear; echo -e "${HDR_BLUE}${BOLD}🚀 Agent Manager 全量同步启动${NC}\n"
        
        # Phase 0: 自动检查并初始化 secrets
        if [ ! -d "$ROOT_DIR/secrets/.git" ]; then
            echo -e "${HDR_BLUE}${BOLD}1. 凭证仓库检查 (Secrets Check)${NC}"
            if [ -n "$OFFLINE_FLAG" ]; then
                status_line "SKIP" "离线模式，跳过 secrets 检查"
            else
                if [ -x "$ROOT_DIR/scripts/init-secrets.sh" ]; then
                    bash "$ROOT_DIR/scripts/init-secrets.sh" || {
                        echo -e "${ERROR_RED}⚠️  secrets 初始化失败，继续执行但 MCP 可能无法正常工作${NC}"
                    }
                fi
            fi
            echo ""
        fi
        
        detect_agents
        echo -e "${HDR_BLUE}${BOLD}2. 任务流水线 (Tasks)${NC}"
        if [ -d "$ROOT_DIR/.git" ] && [ -z "$OFFLINE_FLAG" ]; then
            if out=$(git -C "$ROOT_DIR" pull 2>&1) && [[ "$out" != *"Already up to date"* ]]; then
                status_line "DONE" "逻辑已更新，热重载中..."; exec "$0" "$@"
            else status_line "DONE" "逻辑已是最新"; fi
        elif [ -n "$OFFLINE_FLAG" ]; then
            status_line "SKIP" "离线模式，跳过 git 更新"
        fi
        > "$TEMP_LOG"
        if ! run_task_with_progress "对齐 Agent 生命周期" "生命周期对齐完成" "生命周期失败" timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-lifecycle\" && $PYTHON_CMD sync_agents.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/agents.json\" --yes $DRY_RUN_FLAG $OFFLINE_FLAG"; then TASK_FAILURES=1; fi
        if ! run_task_with_progress "同步全局工具" "工具链同步完成" "工具失败" timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-tool\" && $PYTHON_CMD sync_tools.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/tools.json\" --yes $DRY_RUN_FLAG $OFFLINE_FLAG"; then TASK_FAILURES=1; fi
        if ! run_task_with_progress "同步 Agent Rules" "规则分发完成" "规则同步失败" timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-rule\" && ./scripts/sync-agent-rules.sh $DRY_RUN_FLAG"; then TASK_FAILURES=1; fi
        if ! run_task_with_progress "链接专业技能" "技能链接完成" "技能链接失败" timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-skill\" && $PYTHON_CMD link_skills.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/skills.json\" --yes $DRY_RUN_FLAG"; then TASK_FAILURES=1; fi
        if ! run_task_with_progress "注入 MCP 配置" "MCP 注入完成" "MCP 失败" timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-mcp\" && $PYTHON_CMD sync_mcp.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/mcp.json\" $DRY_RUN_FLAG"; then TASK_FAILURES=1; fi
        show_summary
        [ "$TASK_FAILURES" -eq 0 ] || exit 1
        ;;
    ui) $PYTHON_CMD "$ROOT_DIR/src/interactive.py" ;;
    doctor|status) doctor_check ;;
    detect) detect_agents ;;
    unlock) rm -f "$LOCK_FILE"; echo "锁已解除。" ;;
    logs)
        if [ -f "$TEMP_LOG" ]; then
            if [ -t 1 ] && command -v less >/dev/null 2>&1; then
                less -R "$TEMP_LOG"
            else
                cat "$TEMP_LOG"
            fi
        else
            echo "暂无日志。"
        fi ;;
    clean-logs) rm -f "${TEMP_LOG}"*; echo "日志已清理" ;;
    backups:list) find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) 2>/dev/null | sort -r | head -n 20 || true ;;
    backups:restore) 
        files=$(find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) 2>/dev/null | sort -r | head -n 10 || true)
        if [ -z "$files" ]; then echo "未发现备份。"; exit 0; fi
        select f in $files; do cp -i "$f" "${f%.bak.*}"; echo "已恢复。"; break; done < /dev/tty ;;
    mcp:sync)
        (cd "$ROOT_DIR/agent-mcp" && $PYTHON_CMD sync_mcp.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/mcp.json" $DRY_RUN_FLAG) ;;
    rules:sync)
        (cd "$ROOT_DIR/agent-rule" && ./scripts/sync-agent-rules.sh $DRY_RUN_FLAG) ;;
    skills:link)
        (cd "$ROOT_DIR/agent-skill" && $PYTHON_CMD link_skills.py --config "$CONFIG_FILE" --def-file "$ROOT_DIR/config/skills.json" --yes $DRY_RUN_FLAG) ;;
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
        rm -f /tmp/agent_manager_sync_${USER_NAME}.log*
        echo -e "${SUCCESS_GREEN}全量清理完成。${NC}" ;;
    help|-h|--help|-help)
        echo -e "${HDR_BLUE}${BOLD}Agent Manager 帮助手册 (Platinum Edition)${NC}\n"
        echo -e "${BOLD}${HDR_BLUE}=== 全局参数 (Flags) ===${NC}"
        echo -e "  ${PENDING_GOLD}--dry-run${NC}       空转模式，演示过程日志输出但不产生真实的侧边影响"
        echo -e "  ${PENDING_GOLD}--offline${NC}       离线模式，跳过 Git 同步与在线环境依赖拉取验证，极大提升速度\n"
        echo -e "${BOLD}${HDR_BLUE}=== 日常入口 (Main Flows) ===${NC}"
        echo -e "  ${SUCCESS_GREEN}all${NC}             执行全量同步配置（默认执行，同直接执行 \`make\`）"
        echo -e "  ${SUCCESS_GREEN}ui${NC}              启动基于 Terminal 的交互式的可视化管理终端界面"
        echo -e "  ${SUCCESS_GREEN}doctor / status${NC} 进行深度的资源校验、配置自检与环境排错诊断"
        echo -e "  ${SUCCESS_GREEN}detect${NC}          仅执行各 Agent 开发工具的环境探测与支持情况检测\n"
        echo -e "${BOLD}${HDR_BLUE}=== 运维排错 (Maintenance) ===${NC}"
        echo -e "  ${VAL_PURPLE}cleanup${NC}         执行全量环境大扫除，深度清理临时文件、无用快照缓存及历史日志"
        echo -e "  ${VAL_PURPLE}unlock${NC}          强制清除因意外中断或多进程引起的任务挂起和进程锁"
        echo -e "  ${VAL_PURPLE}backups:list${NC}    查找并列出近期系统中可用的容灾备份配置快照，最多20个"
        echo -e "  ${VAL_PURPLE}backups:restore${NC} 交互式快照选择，用于从特定备份快照中恢复配置"
        echo -e "  ${VAL_PURPLE}logs${NC}            查看系统最近一次执行的完整全链路同步日志"
        echo -e "  ${VAL_PURPLE}clean-logs${NC}      清空由同步流程留存的所有运行日志文件\n"
        echo -e "${BOLD}${HDR_BLUE}=== 分步原子控制 (Atomic syncs) ===${NC}"
        echo -e "  ${BORDER_GRAY}mcp:sync${NC}        独立执行所有环境上下文向开发客户端 (如 Cursor, Windsurf) 的 MCP 注册及注入"
        echo -e "  ${BORDER_GRAY}rules:sync${NC}      独立执行规则与系统级 prompt 指令向各个 Agent 的拷贝或软链接分发"
        echo -e "  ${BORDER_GRAY}skills:link${NC}     独立执行全局或系统级专业技能代码集到 Agent 沙盒空间的映射配置与软链接"
        echo -e "  ${BORDER_GRAY}tools:sync${NC}      独立同步或安装全局统一增强工具链条"
        echo "" ;;
    *) echo -e "${ERROR_RED}错误: 未知命令 '$cmd'${NC}"; exit 1 ;;
esac
