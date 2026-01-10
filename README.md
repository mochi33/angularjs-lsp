# angularjs-lsp

Language Server Protocol (LSP) implementation for AngularJS 1.x applications.

## Features

- **Completion** - Auto-complete AngularJS controllers, services, directives, and methods
- **Go to Definition** - Jump to AngularJS component, service, controller, and directive definitions
- **Find References** - Find all usages of AngularJS symbols across your workspace
- **Hover Information** - Display type and documentation information on hover
- **CodeLens** - Show controller/template relationships with navigation support
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

## Architecture

```
src/
├── main.rs           # Entry point
├── server.rs         # LSP server implementation
├── analyzer/         # AngularJS code analysis
│   ├── angularjs.rs  # AngularJS pattern detection
│   └── parser.rs     # JavaScript parsing (tree-sitter)
├── handlers/         # LSP request handlers
│   ├── completion.rs # Completion provider
│   ├── hover.rs      # Hover provider
│   └── references.rs # References & definition provider
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
