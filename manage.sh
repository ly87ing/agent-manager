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
CONFIG_FILE="$ROOT_DIR/config/manager.json"

# 原子锁检查
if [[ "${1:-}" != "help" && "${1:-}" != "-h" && "${1:-}" != "--help" && "${1:-}" != "-help" && "${1:-}" != "detect" && "${1:-}" != "doctor" && "${1:-}" != "backups:list" && "${1:-}" != "backups:restore" && "${1:-}" != "unlock" ]]; then
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
    trap 'rm -f "$LOCK_FILE"' EXIT
fi

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

_brew_formulae = None
_brew_casks = None
_gh_exts = None

def get_brew_formulae():
    global _brew_formulae
    if _brew_formulae is None:
        if shutil.which('brew'):
            res = subprocess.run('brew list --formula', shell=True, capture_output=True, text=True)
            _brew_formulae = set(res.stdout.split()) if res.returncode == 0 else set()
        else: _brew_formulae = set()
    return _brew_formulae

def get_brew_casks():
    global _brew_casks
    if _brew_casks is None:
        if shutil.which('brew'):
            res = subprocess.run('brew list --cask', shell=True, capture_output=True, text=True)
            _brew_casks = set(res.stdout.split()) if res.returncode == 0 else set()
        else: _brew_casks = set()
    return _brew_casks

def get_gh_exts():
    global _gh_exts
    if _gh_exts is None:
        if shutil.which('gh'):
            res = subprocess.run('gh extension list', shell=True, capture_output=True, text=True)
            _gh_exts = res.stdout if res.returncode == 0 else ''
        else: _gh_exts = ''
    return _gh_exts

def detect_all_providers(bin_name, config):
    found = []
    def get_pkg_name(cmd):
        if not cmd: return None
        m = re.search(r'(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)', cmd)
        return m.group(1) if m else None

    # 1. 物理探测 (兜底)
    p_path = shutil.which(bin_name)
    is_npm = p_path and ('/node/' in p_path or '/npm/' in p_path or p_path.endswith('node') or p_path.endswith('npm'))
    
    # 2. 包管理器探测
    brew_pkg = get_pkg_name(config.get('providers', {}).get('brew', {}).get('install', ''))
    if brew_pkg:
        if brew_pkg in get_brew_formulae(): found.append('brew')
        elif brew_pkg in get_brew_casks(): found.append('brew-cask')
    
    if is_npm: found.append('npm')
    
    ext_name = config.get('ext_name')
    if ext_name:
        out = get_gh_exts()
        if any(line.strip().startswith(ext_name) for line in out.splitlines()): found.append('gh')
    
    # 物理路径或 App 路径探测兜底
    app_path = config.get('app_path')
    if (app_path and os.path.exists(app_path)) or p_path:
        if not found: found.append('system')
    return found

cfg = load('$CONFIG_FILE')
def_a = load('$ROOT_DIR/config/agents.json')
# Agents
for k, v in cfg.get('agents', {}).items():
    if not v.get('enabled', True): continue
    d = def_a.get(k, {})
    all_act = detect_all_providers(d.get('bin',''), d)
    stat = '${SUCCESS_GREEN}✓${NC}' if all_act else '${PENDING_GOLD}○${NC}'
    print(f'   {stat} {k:20} (安装源:{\"/\".join(all_act) if all_act else \"未发现\"})')
# Tools
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

_brew_formulae = None
_brew_casks = None
_gh_exts = None

def get_brew_formulae():
    global _brew_formulae
    if _brew_formulae is None:
        if shutil.which('brew'):
            res = subprocess.run('brew list --formula', shell=True, capture_output=True, text=True)
            _brew_formulae = set(res.stdout.split()) if res.returncode == 0 else set()
        else: _brew_formulae = set()
    return _brew_formulae

def get_brew_casks():
    global _brew_casks
    if _brew_casks is None:
        if shutil.which('brew'):
            res = subprocess.run('brew list --cask', shell=True, capture_output=True, text=True)
            _brew_casks = set(res.stdout.split()) if res.returncode == 0 else set()
        else: _brew_casks = set()
    return _brew_casks

def get_gh_exts():
    global _gh_exts
    if _gh_exts is None:
        if shutil.which('gh'):
            res = subprocess.run('gh extension list', shell=True, capture_output=True, text=True)
            _gh_exts = res.stdout if res.returncode == 0 else ''
        else: _gh_exts = ''
    return _gh_exts

def detect_all_providers(bin_name, config):
    found = []
    def get_pkg_name(cmd):
        if not cmd: return None
        m = re.search(r'(?:install|uninstall)(?:\s+--cask)?\s+([^\s@]+)', cmd)
        return m.group(1) if m else None
    
    p_path = shutil.which(bin_name)
    is_npm = p_path and ('/node/' in p_path or '/npm/' in p_path or p_path.endswith('node') or p_path.endswith('npm'))

    brew_pkg = get_pkg_name(config.get('providers', {}).get('brew', {}).get('install', ''))
    if brew_pkg:
        if brew_pkg in get_brew_formulae(): found.append('brew')
        elif brew_pkg in get_brew_casks(): found.append('brew-cask')
    
    if is_npm: found.append('npm')
    ext_name = config.get('ext_name')
    if ext_name:
        out = get_gh_exts()
        if any(line.strip().startswith(ext_name) for line in out.splitlines()): found.append('gh')
    
    # 物理路径或 App 路径探测兜底
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
case "${1:-}" in
    --dry-run) cmd="all"; DRY_RUN_FLAG="--dry-run"; shift ;;
    *) if [ $# -eq 0 ]; then cmd="all"; else cmd="$1"; shift; fi
       [[ "$*" == *"--dry-run"* ]] && DRY_RUN_FLAG="--dry-run" ;;
esac

case "$cmd" in
    all|"")
        clear; echo -e "${HDR_BLUE}${BOLD}🚀 Agent Manager 全量同步启动${NC}\n"
        detect_agents
        echo -e "${HDR_BLUE}${BOLD}2. 任务流水线 (Tasks)${NC}"
        if [ -d "$ROOT_DIR/.git" ]; then
            if out=$(git -C "$ROOT_DIR" pull 2>&1) && [[ "$out" != *"Already up to date"* ]]; then
                status_line "DONE" "逻辑已更新，热重载中..."; exec "$0" "$@"
            else status_line "DONE" "逻辑已是最新"; fi
        fi
        > "$TEMP_LOG"
        status_line "RUNNING" "对齐 Agent 生命周期..."
        (timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-lifecycle\" && $PYTHON_CMD sync_agents.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/agents.json\" --yes $DRY_RUN_FLAG" >> "$TEMP_LOG" 2>&1) && status_line "DONE" "生命周期对齐完成" || { status_line "ERROR" "生命周期失败"; echo "FAILED: agents" >> "$TEMP_LOG"; }
        status_line "RUNNING" "同步全局工具..."
        (timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-tool\" && $PYTHON_CMD sync_tools.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/tools.json\" --yes $DRY_RUN_FLAG" >> "$TEMP_LOG" 2>&1) && status_line "DONE" "工具链同步完成" || { status_line "ERROR" "工具失败"; echo "FAILED: tools" >> "$TEMP_LOG"; }
        status_line "RUNNING" "同步 Agent Rules..."
        (timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-rule\" && ./scripts/sync-agent-rules.sh $DRY_RUN_FLAG" >> "$TEMP_LOG" 2>&1) && status_line "DONE" "规则分发完成" || { status_line "ERROR" "规则同步失败"; echo "FAILED: rules" >> "$TEMP_LOG"; }
        status_line "RUNNING" "链接专业技能..."
        (timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-skill\" && $PYTHON_CMD link_skills.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/skills.json\" --yes $DRY_RUN_FLAG" >> "$TEMP_LOG" 2>&1) && status_line "DONE" "技能链接完成" || { status_line "ERROR" "技能链接失败"; echo "FAILED: skills" >> "$TEMP_LOG"; }
        status_line "RUNNING" "注入 MCP 配置..."
        (timeout_run 600 bash -c "cd \"$ROOT_DIR/agent-mcp\" && $PYTHON_CMD sync_mcp.py --config \"$CONFIG_FILE\" --def-file \"$ROOT_DIR/config/mcp.json\" $DRY_RUN_FLAG" >> "$TEMP_LOG" 2>&1) && status_line "DONE" "MCP 注入完成" || { status_line "ERROR" "MCP 失败"; echo "FAILED: mcp" >> "$TEMP_LOG"; }
        show_summary
        ;;
    ui) $PYTHON_CMD "$ROOT_DIR/src/interactive.py" ;;
    doctor|status) doctor_check ;;
    detect) detect_agents ;;
    unlock) rm -f "$LOCK_FILE"; echo "锁已解除。" ;;
    backups:list) find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) | sort -r | head -n 20 ;;
    backups:restore) 
        files=$(find "$HOME" -maxdepth 3 \( -name "*.bak.*" -o -name "*.bak" \) | sort -r | head -n 10)
        if [ -z "$files" ]; then echo "未发现备份。"; exit 0; fi
        select f in $files; do cp -i "$f" "${f%.bak.*}"; echo "已恢复。"; break; done < /dev/tty ;;
    cleanup)
        echo -e "${HDR_BLUE}正在执行全量环境大扫除...${NC}"
        (cd "$ROOT_DIR/agent-rule" && ./scripts/clean-agent-rule-backups.sh --yes || true)
        local agents_paths=$($PYTHON_CMD -c "import json,os; def get_p(v): return os.path.expanduser(v if isinstance(v,str) else v.get('path','')); try: m=json.load(open('$ROOT_DIR/config/mcp.json')).get('agent_paths',{}).values(); s=json.load(open('$ROOT_DIR/config/skills.json')).get('agents',{}).values(); print('\n'.join({os.path.dirname(get_p(p)) for p in list(m)+list(s) if p})) except: pass")
        echo "$agents_paths" | while read -r d; do [ -d "$d" ] && find "$d" -maxdepth 1 \( -name "*.bak.*" -o -name "*.tmp_*" \) -delete 2>/dev/null || true; done
        rm -f /tmp/agent_manager_sync_${USER_NAME}.log*
        echo -e "${SUCCESS_GREEN}全量清理完成。${NC}" ;;
    help|-h|--help|-help)
        echo -e "${HDR_BLUE}Agent Manager 帮助手册 (Platinum Edition)${NC}"
        echo "常用命令: all, ui, doctor, detect, cleanup, backups:list, backups:restore" ;;
    *) echo -e "${ERROR_RED}错误: 未知命令 '$1'${NC}"; exit 1 ;;
esac
