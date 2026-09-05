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
# If several check in and stdin is a TTY, you are prompted with inquire.
# If several check in and stdin is not a TTY, the CLI prints the list and exits 2.
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

Resolution order (applied when resolving):

1. Exact GUID (the `place-guid` map key)
2. Unique place id (`place-id` as a decimal string)
3. Unique place name, case-sensitive
4. Unique place name, ASCII case-insensitive

Wait rules:

- Exact GUID, or a unique numeric place id that is already present: select immediately. More check-ins cannot change that match, so agents do not need to wait out `--timeout`.
- Name matches (case-sensitive or ASCII case-insensitive): wait the full `--timeout` (same as `--list`) collecting places, then resolve once. A unique name mid-wait is not selected early, because a second Studio instance with the same name may still check in.
- After that wait: 0 matches → exit 2; more than one match → exit 2 and print the connected places; exactly one → activate.

If nothing matches, or more than one place matches after name/id resolution, the CLI prints the connected places and exits with code 2. `--place` never calls the interactive prompt.

### Non-TTY (agents / CI)

The default path uses inquire only when stdin is a TTY. If more than one place is connected and stdin is not a TTY, the CLI prints the connected places, tells you to pass `--place`, and exits 2 so the process (and port 28859) is released. Prompt errors, including inquire `NotTTY`, do the same: a message on stderr, the connected list, and process exit 2. A single connected place is still auto-selected without a prompt.

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
| 2 | No places checked in, `--list` was empty after the timeout, `--place` did not uniquely match, or interactive selection failed (non-TTY / prompt error) |

If you wanted, you could add this to a pre-commit hook to ensure that tests pass before committing (though, it would require your place be open in Studio, of course). Agents can use `--list --json` then `--place <guid>` instead of the interactive prompt.
