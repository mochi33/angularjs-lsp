# angularjs-lsp

Language Server Protocol (LSP) implementation for AngularJS 1.x applications.

## Features

- **Completion** - Auto-complete AngularJS controllers, services, directives, and methods
  - Context-aware: Controllers are excluded from completions when inside a controller
  - `$scope.` completions show only properties/methods from the current controller
  - Service method completions with `ServiceName.` prefix
- **Go to Definition** - Jump to AngularJS component, service, controller, and directive definitions
- **Find References** - Find all usages of AngularJS symbols across your workspace
- **Hover Information** - Display type and documentation information on hover
- **Signature Help** - Display function parameter hints while typing
- **CodeLens** - Show controller/template relationships with navigation support
- **Workspace Symbol** - Search AngularJS symbols across the workspace (`Ctrl+T` / `Cmd+T`)
- **Diagnostics** - Show warnings for undefined scope properties and local variables in HTML templates
- **TypeScript Fallback** - Automatically falls back to `typescript-language-server` for non-AngularJS symbols

## Supported AngularJS Constructs

- Controllers (`app.controller()`)
- Services (`app.service()`, `app.factory()`)
- Directives (`app.directive()`)
- Components (`app.component()`)
- Modules (`angular.module()`)

## Installation

### Building from Source

```bash
git clone https://github.com/mochi33/angularjs-lsp.git
cd angularjs-lsp
cargo build --release
```

The binary will be located at `target/release/angularjs-lsp`.

### Prerequisites

- Rust 1.70+
- (Optional) `typescript-language-server` for fallback support

## Editor Setup

### Neovim (nvim-lspconfig)

```lua
local lspconfig = require('lspconfig')
local configs = require('lspconfig.configs')

if not configs.angularjs_lsp then
  configs.angularjs_lsp = {
    default_config = {
      cmd = { '/path/to/angularjs-lsp' },
      filetypes = { 'javascript', 'html' },
      root_dir = lspconfig.util.root_pattern('package.json', '.git'),
    },
  }
end

lspconfig.angularjs_lsp.setup({})
```

#### Custom Commands

CodeLens uses the `angularjs.openLocation` command for navigation. Add this handler to enable CodeLens click-to-jump:

```lua
vim.lsp.commands["angularjs.openLocation"] = function(command, ctx)
  local locations = command.arguments[1]

  if #locations == 0 then
    return
  elseif #locations == 1 then
    -- Single location: jump directly
    local loc = locations[1]
    vim.cmd("edit " .. vim.uri_to_fname(loc.uri))
    vim.api.nvim_win_set_cursor(0, { loc.range.start.line + 1, loc.range.start.character })
  else
    -- Multiple locations: show selection UI
    vim.ui.select(locations, {
      prompt = "Select location:",
      format_item = function(loc)
        return vim.fn.fnamemodify(vim.uri_to_fname(loc.uri), ":t")
      end,
    }, function(selected)
      if selected then
        vim.cmd("edit " .. vim.uri_to_fname(selected.uri))
        vim.api.nvim_win_set_cursor(0, { selected.range.start.line + 1, selected.range.start.character })
      end
    end)
  end
end
```

### VS Code

1. Build the extension:
   ```bash
   cd vscode-extension
   npm install
   npm run compile
   ```

2. In VSCode, open the `vscode-extension` folder and press F5 to launch Extension Development Host, or package as VSIX:
   ```bash
   npm run package
   ```

3. Configure the server path in VS Code settings:
   ```json
   {
     "angularjsLsp.serverPath": "/path/to/angularjs-lsp/target/release/angularjs-lsp"
   }
   ```

## Configuration

Create an `ajsconfig.json` file in your project root to customize the language server behavior.

```json
{
  "include": ["src/**/*.js", "app/**/*.js"],
  "exclude": ["**/test/**", "**/vendor/**"],
  "interpolate": {
    "startSymbol": "{{",
    "endSymbol": "}}"
  },
  "cache": true,
  "diagnostics": {
    "enabled": true,
    "severity": "warning"
  }
}
```

### Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `include` | `string[]` | `[]` (all files) | Glob patterns for files to analyze. If empty, all files are included. |
| `exclude` | `string[]` | (see below) | Glob patterns for files/directories to exclude. |
| `interpolate.startSymbol` | `string` | `{{` | AngularJS interpolation start symbol. |
| `interpolate.endSymbol` | `string` | `}}` | AngularJS interpolation end symbol. |
| `cache` | `boolean` | `true` | Enable caching of parsed symbols. Cache is stored in `.angularjs-lsp/cache/`. |
| `diagnostics.enabled` | `boolean` | `true` | Enable diagnostics for undefined scope properties and local variables. |
| `diagnostics.severity` | `string` | `"warning"` | Severity level: `"error"`, `"warning"`, `"hint"`, or `"information"`. |

### Default Exclude Patterns

By default, the following patterns are excluded:

- `**/node_modules` and `**/node_modules/**`
- `**/dist` and `**/dist/**`
- `**/build` and `**/build/**`
- `**/.*` and `**/.*/**` (hidden files/directories)

### Example Configurations

**Only analyze specific directories:**
```json
{
  "include": ["src/**/*.js", "app/**/*.html"]
}
```

**Exclude test files:**
```json
{
  "exclude": ["**/test/**", "**/spec/**", "**/*.spec.js", "**/*.test.js"]
}
```

**Custom interpolation symbols (e.g., for ERB/Jinja compatibility):**
```json
{
  "interpolate": {
    "startSymbol": "[[",
    "endSymbol": "]]"
  }
}
```

**Disable diagnostics or change severity:**
```json
{
  "diagnostics": {
    "enabled": false
  }
}
```

```json
{
  "diagnostics": {
    "severity": "hint"
  }
}
```

## Architecture

```
src/
├── main.rs           # Entry point
├── server.rs         # LSP server implementation
├── analyzer/         # AngularJS code analysis
│   ├── angularjs.rs  # AngularJS pattern detection
│   └── parser.rs     # JavaScript parsing (tree-sitter)
├── handlers/         # LSP request handlers
│   ├── completion.rs     # Completion provider
│   ├── hover.rs          # Hover provider
│   ├── references.rs     # References & definition provider
│   ├── signature_help.rs # Signature help provider
│   ├── codelens.rs       # CodeLens provider
│   ├── document_symbol.rs  # Document symbol provider
│   ├── workspace_symbol.rs # Workspace symbol provider
│   └── rename.rs           # Rename provider
├── index/            # Symbol indexing
│   ├── store.rs      # In-memory symbol store
│   └── symbol.rs     # Symbol data structures
└── ts_proxy/         # TypeScript LSP proxy
    ├── transport.rs  # JSON-RPC transport
    └── mod.rs        # Proxy implementation
```

## Development

```bash
# Run with debug logging
RUST_LOG=info,angularjs_lsp=debug cargo run

# Run tests
cargo test

# Build release
cargo build --release
```

## License

MIT
