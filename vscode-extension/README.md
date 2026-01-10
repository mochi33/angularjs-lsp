# AngularJS Language Server - VSCode Extension

Language Server Protocol implementation for AngularJS 1.x projects.

## Features

- **Completion**: Autocomplete for AngularJS components, services, controllers, and $scope properties
- **Hover**: Documentation and type information on hover
- **Go to Definition**: Navigate to AngularJS component definitions
- **Find References**: Find all references to AngularJS symbols
- **Rename**: Rename AngularJS symbols across your project
- **Document Symbols**: Outline view showing AngularJS components
- **Code Lens**: Shows controller-template bindings

## Requirements

- AngularJS 1.x project
- The `angularjs-lsp` server binary (build from source)

## Installation

1. Install the extension (from VSIX or Extension Development Host)
2. Build the `angularjs-lsp` server binary:
   ```bash
   cd /path/to/angularjs-lsp
   cargo build --release
   ```
3. Configure the server path in VS Code settings

## Configuration

Configure the path to the LSP server binary in your VS Code settings:

```json
{
  "angularjsLsp.serverPath": "/path/to/angularjs-lsp/target/release/angularjs-lsp"
}
```

### Settings

| Setting | Type | Default | Description |
|---------|------|---------|-------------|
| `angularjsLsp.serverPath` | string | `""` | Path to the angularjs-lsp server executable |
| `angularjsLsp.trace.server` | string | `"off"` | Traces communication between VS Code and the server |

## Project Configuration

Create an `ajsconfig.json` file in your project root to customize behavior:

```json
{
  "interpolate": {
    "startSymbol": "{{",
    "endSymbol": "}}"
  }
}
```

## Commands

- `AngularJS: Restart Language Server` - Restart the language server

## License

MIT
