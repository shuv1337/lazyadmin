# CLI reference

lazyadmin is a Clap-based CLI with global flags and subcommands.
Every command supports `--json` for machine-readable output.

## Global options

```
Usage: lazyadmin [OPTIONS] [COMMAND]

Options:
      --json                     JSON output
      --brief                    Brief output
      --config <CONFIG>          Config file path
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               Increase verbosity
  -h, --help                     Print help
  -V, --version                  Print version
```

## Command summary

| Command | Purpose | JSON schema |
|---------|---------|-------------|
| `web` | Start read-only Web UI server | n/a |
| `port` | Explain a specific port | lazyadmin.snapshot.v1 |
| `free` | Safely free a port | n/a |
| `ps` | List all listeners | lazyadmin.snapshot.v1 |
| `public` | List public/LAN-facing listeners | lazyadmin.snapshot.v1 |
| `conflicts` | List contended ports | lazyadmin.snapshot.v1 |
| `projects` | List detected projects | lazyadmin.snapshot.v1 |
| `overview` | High-level digest | lazyadmin.digest.v1 |
| `logs` | Show systemd journal logs | n/a |
| `doctor` | Run subsystem health checks | lazyadmin.doctor.v1 |
| `events` | Stream discovery events | lazyadmin.discovery_event.v1 |
| `export` | Export full snapshot | lazyadmin.snapshot.v1 |
| `diff` | Compare two snapshots | lazyadmin.diff.v1 |
| `search` | Search across entities | lazyadmin.search.v1 |
| `run` | Wrap a command as a tracked run | n/a |
| `runs` | List tracked runs | lazyadmin.tracked_runs.v1 |
| `pause-restart` | Pause auto-restart policy | n/a |
| `resume-restart` | Resume auto-restart policy | n/a |
| `config` | Configuration commands | n/a |

## Per-command reference

### web

```
Usage: lazyadmin web [OPTIONS]

Options:
      --bind <BIND>              [default: 127.0.0.1]
      --json                     
      --brief                    
      --port <PORT>              [default: 7749]
      --config <CONFIG>          
      --no-open                  
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
      --refresh-ms <REFRESH_MS>  [default: 2000]
  -v, --verbose...               
  -h, --help                     Print help
```

### port

JSON schema: `lazyadmin.snapshot.v1`

```
Usage: lazyadmin port [OPTIONS] <PORT>

Arguments:
  <PORT>  

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### free

```
Usage: lazyadmin free [OPTIONS] <PORT>

Arguments:
  <PORT>  

Options:
      --dry-run                  
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### ps

JSON schema: `lazyadmin.snapshot.v1`

```
Usage: lazyadmin ps [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### public

JSON schema: `lazyadmin.snapshot.v1`

```
Usage: lazyadmin public [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### conflicts

JSON schema: `lazyadmin.snapshot.v1`

```
Usage: lazyadmin conflicts [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### projects

JSON schema: `lazyadmin.snapshot.v1`

```
Usage: lazyadmin projects [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### overview

JSON schema: `lazyadmin.digest.v1`

```
Usage: lazyadmin overview [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### logs

```
Usage: lazyadmin logs [OPTIONS] <SELECTOR>

Arguments:
  <SELECTOR>  

Options:
      --json                     
      --tail <TAIL>              
      --brief                    
      --follow                   
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### doctor

JSON schema: `lazyadmin.doctor.v1`

```
Usage: lazyadmin doctor [OPTIONS]

Options:
      --groups                   
      --json                     
      --all                      
      --brief                    
      --actionable               
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### events

JSON schema: `lazyadmin.discovery_event.v1`

```
Usage: lazyadmin events [OPTIONS]

Options:
      --json                     
      --once                     
      --brief                    
      --follow                   
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### export

JSON schema: `lazyadmin.snapshot.v1`

```
Usage: lazyadmin export [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### diff

JSON schema: `lazyadmin.diff.v1`

```
Usage: lazyadmin diff [OPTIONS] <BEFORE> <AFTER>

Arguments:
  <BEFORE>  
  <AFTER>   

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### search

JSON schema: `lazyadmin.search.v1`

```
Usage: lazyadmin search [OPTIONS] <QUERY>

Arguments:
  <QUERY>  The search query (text, port number, or PID)

Options:
      --json                     
      --kind <KIND>              Filter results to a specific entity kind [possible values: listeners, processes, workloads, projects, managers, all]
      --brief                    
      --limit <LIMIT>            Maximum number of results per group
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### run

```
Usage: lazyadmin run [OPTIONS] [CMD]...

Arguments:
  [CMD]...  

Options:
      --json                     
      --tag <TAG>                
      --brief                    
      --detach                   
      --config <CONFIG>          
      --cwd <CWD>                
      --env <ENVS>               
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### runs

JSON schema: `lazyadmin.tracked_runs.v1`

```
Usage: lazyadmin runs [OPTIONS]

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### pause-restart

```
Usage: lazyadmin pause-restart [OPTIONS] <SELECTOR>

Arguments:
  <SELECTOR>  

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### resume-restart

```
Usage: lazyadmin resume-restart [OPTIONS] <SELECTOR>

Arguments:
  <SELECTOR>  

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

### config

```
Usage: lazyadmin config [OPTIONS] <COMMAND>

Commands:
  check  
  help   Print this message or the help of the given subcommand(s)

Options:
      --json                     
      --brief                    
      --config <CONFIG>          
      --log-format <LOG_FORMAT>  [default: text] [possible values: text, json]
  -v, --verbose...               
  -h, --help                     Print help
```

## JSON output

Every command that produces structured data supports `--json`.
The output uses stable schema versions such as `lazyadmin.snapshot.v1`,
`lazyadmin.doctor.v1`, `lazyadmin.diff.v1`, and `lazyadmin.discovery_event.v1`.

Schema documentation:
- [Snapshot v1](schema/snapshot-v1.md)
- [Diff v1](schema/diff-v1.md)
- [Doctor v1](schema/doctor-v1.md)

## Examples

### Health check

```bash
lazyadmin doctor
lazyadmin doctor --json
```

### Export and diff

```bash
lazyadmin export --json > before.json
# ... make changes ...
lazyadmin export --json > after.json
lazyadmin diff before.json after.json --json
```

### Watch events

```bash
lazyadmin events --once --json
lazyadmin events --json | jq .
```

### Web UI headless

```bash
lazyadmin web --port 0 --no-open
```
