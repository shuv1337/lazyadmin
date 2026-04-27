# Example

Use `lazyadmin doctor --json` first. Prefer JSON output. If changing runtime state, capture `lazyadmin export --json` before and `lazyadmin diff` after. Do not use unsafe first-line kill patterns.

```bash
command -v lazyadmin && lazyadmin doctor --json
lazyadmin export --json > /tmp/lazyadmin-before.json
# perform the specific task safely
lazyadmin diff /tmp/lazyadmin-before.json - --json
```
