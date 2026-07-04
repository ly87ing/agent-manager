import os

replacements = {
    "make bootstrap": "make init",
    "make update": "make bump",
    "make sync": "make apply",
    "make rules:sync": "make rules:apply",
    "manage.sh rules:sync": "manage.sh rules:apply",
    "make skills:link": "make skills:apply",
    "make skills:upgrade": "make skills:bump",
    "skills_upgrade": "skills:bump",
    "make mcp:sync": "make mcp:apply",
    "make tools:sync": "make tools:apply",
    "make self-update": "make sys:upgrade",
    "make cleanup": "make sys:clean",
    "make logs": "make sys:logs",
    "make backups:restore": "make state:restore",
    "make backups:list": "make state:list",
    "make backups-gc": "make state:clean",
    "make unlock": "make sys:unlock",
    "make clean-logs": "make sys:clean-logs",
}

for root, dirs, files in os.walk('.'):
    # Skip some dirs
    if any(d in root for d in ['.git', 'node_modules', '__pycache__', 'agent-rule/rules/dist']):
        continue
    for file in files:
        if file in ['Makefile', 'manage.sh', 'fix_docs.py', 'rename.py']:
            continue
        if file.endswith(('.md', '.py', '.sh')):
            filepath = os.path.join(root, file)
            try:
                with open(filepath, 'r') as f:
                    content = f.read()
            except Exception:
                continue
            
            new_content = content
            for old, new in replacements.items():
                new_content = new_content.replace(old, new)
            
            if new_content != content:
                with open(filepath, 'w') as f:
                    f.write(new_content)
                print(f"Updated {filepath}")
