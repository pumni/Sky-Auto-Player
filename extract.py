import re
import os

with open('docs/2026-07-18_distribution-mpv-pattern-plan.md', encoding='utf-8') as f:
    md = f.read()

# Find the block starting after `### 2.6`
start_idx = md.find('### 2.6 `installer/updater.ps1`')
md = md[start_idx:]

match = re.search(r'```powershell\n(.*?)```', md, re.DOTALL)
if match:
    os.makedirs('installer', exist_ok=True)
    with open('installer/updater.ps1', 'w', encoding='utf-8') as f:
        f.write(match.group(1))
    print("Success")
else:
    print("Failed to find block")
