# AngularJS Language Server

Language Server Protocol implementation for AngularJS 1.x projects.

## Features

- **Auto Completion** - IntelliSense for $scope properties, services, controllers, and directives
- **Go to Definition** - Navigate to AngularJS component definitions
- **Find References** - Find all usages of AngularJS components
- **Hover Information** - Documentation and type information on hover
- **Signature Help** - Parameter hints for function calls
- **Rename Symbol** - Safely rename AngularJS components across files
- **Document Symbols** - Outline view for AngularJS components
- **Code Lens** - Reference counts and controller-template bindings

## Supported AngularJS Components

- Controllers (including `controller as` syntax)
- Services
- Factories
- Directives
- Components
- $scope properties and methods

## Installation

The extension automatically downloads and manages the language server binary. No manual setup required!

On first activation:
1. **angularjs-lsp** binary is downloaded from GitHub releases
2. **typescript-language-server** is installed via npm (for JavaScript fallback)

## Configuration

### `angularjsLsp.serverPath`

Path to the angularjs-lsp server executable. If empty (default), the extension automatically downloads and manages the server binary.

### `angularjsLsp.trace.server`

Traces the communication between VS Code and the AngularJS language server.
- `off` (default)
- `messages`
- `verbose`

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

- **AngularJS: Restart Language Server** - Restart the language server

## Updates

The extension automatically checks for new versions of angularjs-lsp and prompts you to update when a new release is available.

## Requirements

- VS Code 1.75.0 or higher
- Node.js (for typescript-language-server fallback)

## License

MIT

## Links

- [GitHub Repository](https://github.com/mochi33/angularjs-lsp)
- [Report Issues](https://github.com/mochi33/angularjs-lsp/issues)
