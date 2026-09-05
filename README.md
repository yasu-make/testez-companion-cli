# TestEZ Companion CLI

> [!WARNING]
> This isn't well tested and was made in a couple hours. If something doesn't work, please [file an issue](https://github.com/jackTabsCode/testez-companion-cli/issues).

This is a quick and dirty CLI to interface with [TestEZ Companion](https://github.com/tacheometry/testez-companion). It was made to allow users to run tests from the command line without needing to use the VSCode extension.

You'll need the plugin for this to work, which is [bundled in the latest release](https://github.com/jackTabsCode/testez-companion-cli/releases/latest/download/TestEZ_Companion.rbxm) (alternatively, you can [build it from source](https://github.com/tacheometry/testez-companion/tree/main/plugin)).

![SCR-20240421-cxmd](https://github.com/jackTabsCode/testez-companion-cli/assets/44332148/246a6cd6-5b65-47a1-8c74-9baa7487448e)

## Features

-   Easily run tests from the command line
-   Supports multiple places
-   Pretty prints results
-   Prints logs and other output from Studio
-   ~~Installs the plugin for you~~ (not implemented yet)

## Installation

### Aftman

```sh
aftman add jacktabscode/testez-companion-cli
```

### Cargo

```sh
cargo install testez-companion-cli
```

## Usage

```sh
# Interactive (default): wait for Studio places.
# If one place checks in, it is selected automatically.
# If several check in, you are prompted with inquire.
testez-companion-cli

# List connected places and exit without running tests
testez-companion-cli --list
testez-companion-cli --list --json
testez-companion-cli --list --timeout 10

# Non-interactive place selection (never prompts)
testez-companion-cli --place <guid|name|id>

# Only print failing tests
testez-companion-cli --only-print-failures
```

`--list` and `--place` start the same local server and wait for Studio plugins to check in on `GET /poll` (they do not select an active place when listing). `--timeout` controls that wait; the default is 5 seconds, matching the previous hardcoded value.

### Place matching (`--place`)

Resolution order:

1. Exact GUID (the `place-guid` map key)
2. Unique place id (`place-id` as a decimal string)
3. Unique place name, case-sensitive
4. Unique place name, ASCII case-insensitive

If nothing matches, or more than one place matches after name/id resolution, the CLI prints the connected places and exits with code 2. `--place` never calls the interactive prompt.

### JSON (`--json`)

With `--list`, stdout is a JSON array:

```json
[{"guid":"...","name":"...","id":123}]
```

When running tests, `--json` also prints a final summary on stdout:

```json
{"success":true,"successCount":1,"failureCount":0,"skippedCount":0}
```

Status messages, the pretty test tree, and Studio logs go to stderr when `--json` is set so agents can parse stdout.

### Exit codes

| Code | Meaning |
| ---- | ------- |
| 0 | Success (`--list` found places, or tests passed) |
| 1 | Tests ran and one or more failed |
| 2 | No places checked in, `--list` was empty after the timeout, or `--place` did not uniquely match |

If you wanted, you could add this to a pre-commit hook to ensure that tests pass before committing (though, it would require your place be open in Studio, of course). Agents can use `--list --json` then `--place <guid>` instead of the interactive prompt.
