import re

# Update Makefile
with open('Makefile', 'r') as f:
    mf = f.read()

mf = mf.replace('\nbootstrap:\n\t@./manage.sh bootstrap', '\ninit bootstrap:\n\t@./manage.sh init')
mf = mf.replace('\ninstall: bootstrap', '\ninstall: init')
mf = mf.replace('\nsync:\n\t@./manage.sh sync', '\napply sync:\n\t@./manage.sh apply')
mf = mf.replace('\nupdate:\n\t@./manage.sh update', '\nbump update:\n\t@./manage.sh bump')
mf = mf.replace('\nself-update:\n\t@./manage.sh self-update', '\nsys-upgrade sys\:upgrade self-update:\n\t@./manage.sh sys:upgrade')
mf = mf.replace('\ncleanup:\n\t@./manage.sh cleanup', '\nsys-clean sys\:clean cleanup:\n\t@./manage.sh sys:clean')
mf = mf.replace('\nbackups-gc:\n\t@find ~/.claude ~/.codex ~/.gemini -maxdepth 2 -type f -name \'*.bak.*\' -mtime +30 -print -delete 2>/dev/null || true', '\nstate-clean state\:clean backups-gc:\n\t@find ~/.claude ~/.codex ~/.gemini -maxdepth 2 -type f -name \'*.bak.*\' -mtime +30 -print -delete 2>/dev/null || true')
mf = mf.replace('\nunlock:\n\t@./manage.sh unlock', '\nsys-unlock sys\:unlock unlock:\n\t@./manage.sh sys:unlock')
mf = mf.replace('\nbackups-list:\n\t@./manage.sh backups:list\n\nbackups\\:list:\n\t@./manage.sh backups:list', '\nstate-list state\:list backups-list backups\\:list:\n\t@./manage.sh state:list')
mf = mf.replace('\nrestore:\n\t@./manage.sh backups:restore\n\nbackups\\:restore:\n\t@./manage.sh backups:restore', '\nstate-restore state\:restore restore backups\\:restore:\n\t@./manage.sh state:restore')
mf = mf.replace('\nlogs:\n\t@./manage.sh logs', '\nsys-logs sys\:logs logs:\n\t@./manage.sh sys:logs')
mf = mf.replace('\nclean-logs:\n\t@./manage.sh clean-logs', '\nsys-clean-logs sys\:clean-logs clean-logs:\n\t@./manage.sh sys:clean-logs')

# Resources
mf = mf.replace('rules-sync:\n\t@./manage.sh rules:sync\n\nrules\\:sync:\n\t@./manage.sh rules:sync', 'rules-apply rules\:apply rules-sync rules\\:sync:\n\t@./manage.sh rules:apply')
mf = mf.replace('skills-link:\n\t@./manage.sh skills:link\n\nskills\\:link:\n\t@./manage.sh skills:link', 'skills-apply skills\:apply skills-link skills\\:link:\n\t@./manage.sh skills:apply')
mf = mf.replace('skills\\:upgrade:\n\t@./manage.sh skills:upgrade', 'skills-bump skills\:bump skills\\:upgrade:\n\t@./manage.sh skills:bump')
mf = mf.replace('mcp-sync:\n\t@./manage.sh mcp:sync\n\nmcp\\:sync:\n\t@./manage.sh mcp:sync', 'mcp-apply mcp\:apply mcp-sync mcp\\:sync:\n\t@./manage.sh mcp:apply')
mf = mf.replace('mcp-prepare:\n\t@./manage.sh mcp:prepare\n\nmcp\\:prepare:\n\t@./manage.sh mcp:prepare', 'mcp-build mcp\:build mcp-prepare mcp\\:prepare:\n\t@./manage.sh mcp:build')
mf = mf.replace('tools-sync:\n\t@./manage.sh tools:sync\n\ntools\\:sync:\n\t@./manage.sh tools:sync', 'tools-apply tools\:apply tools-sync tools\\:sync:\n\t@./manage.sh tools:apply')
mf = mf.replace('browser-us-sync:\n\t@./manage.sh browser:sync\n\nbrowser\\:sync:\n\t@./manage.sh browser:sync', 'browser-apply browser\:apply browser-us-sync browser\\:sync:\n\t@./manage.sh browser:apply')
mf = mf.replace('claude-hooks-sync:\n\t@./manage.sh claude:hooks:sync\n\nclaude\\:hooks\\:sync:\n\t@./manage.sh claude:hooks:sync', 'claude-hooks-apply claude\:hooks\:apply claude-hooks-sync claude\\:hooks\\:sync:\n\t@./manage.sh claude:hooks:apply')
mf = mf.replace('claude-shell-aliases-sync:\n\t@./manage.sh claude:shell-aliases:sync\n\nclaude\\:shell-aliases\\:sync:\n\t@./manage.sh claude:shell-aliases:sync', 'claude-shell-aliases-apply claude\:shell-aliases\:apply claude-shell-aliases-sync claude\\:shell-aliases\\:sync:\n\t@./manage.sh claude:shell-aliases:apply')
mf = mf.replace('claude-statusline-install:\n\t@./manage.sh claude:statusline:install\n\nclaude\\:statusline\\:install:\n\t@./manage.sh claude:statusline:install', 'claude-statusline-apply claude\:statusline\:apply claude-statusline-install claude\\:statusline\\:install:\n\t@./manage.sh claude:statusline:apply')
mf = mf.replace('codex-hooks-sync:\n\t@./manage.sh codex:hooks:sync\n\ncodex\\:hooks\\:sync:\n\t@./manage.sh codex:hooks:sync', 'codex-hooks-apply codex\:hooks\:apply codex-hooks-sync codex\\:hooks\\:sync:\n\t@./manage.sh codex:hooks:apply')

with open('Makefile', 'w') as f:
    f.write(mf)

# Update manage.sh
with open('manage.sh', 'r') as f:
    ms = f.read()

ms = ms.replace('case "$cmd" in\n    all|bootstrap)', 'case "$cmd" in\n    all|bootstrap|init)')
ms = ms.replace('\n    update)', '\n    bump|update)')
ms = ms.replace('\n    sync)', '\n    apply|sync)')
ms = ms.replace('\n    self-update)', '\n    sys:upgrade|self-update)')
ms = ms.replace('\n    unlock)', '\n    sys:unlock|unlock)')
ms = ms.replace('\n    logs)', '\n    sys:logs|logs)')
ms = ms.replace('\n    clean-logs)', '\n    sys:clean-logs|clean-logs)')
ms = ms.replace('\n    cleanup)', '\n    sys:clean|cleanup)')
ms = ms.replace('\n    backups:list)', '\n    state:list|backups:list)')
ms = ms.replace('\n    backups:restore)', '\n    state:restore|backups:restore)')
ms = ms.replace('\n    mcp:sync)', '\n    mcp:apply|mcp:sync)')
ms = ms.replace('\n    mcp:prepare)', '\n    mcp:build|mcp:prepare)')
ms = ms.replace('\n    rules:sync)', '\n    rules:apply|rules:sync)')
ms = ms.replace('\n    claude:hooks:sync)', '\n    claude:hooks:apply|claude:hooks:sync)')
ms = ms.replace('\n    claude:shell-aliases:sync)', '\n    claude:shell-aliases:apply|claude:shell-aliases:sync)')
ms = ms.replace('\n    claude:statusline:install)', '\n    claude:statusline:apply|claude:statusline:install)')
ms = ms.replace('\n    codex:hooks:sync)', '\n    codex:hooks:apply|codex:hooks:sync)')
ms = ms.replace('\n    skills:link)', '\n    skills:apply|skills:link)')
ms = ms.replace('\n    skills:upgrade)', '\n    skills:bump|skills:upgrade)')
ms = ms.replace('\n    tools:sync)', '\n    tools:apply|tools:sync)')
ms = ms.replace('\n    browser:sync)', '\n    browser:apply|browser:sync)')

with open('manage.sh', 'w') as f:
    f.write(ms)
