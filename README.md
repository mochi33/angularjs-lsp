# angularjs-lsp

Language Server Protocol (LSP) implementation for AngularJS 1.x applications.

## Features

- **Completion** - Auto-complete AngularJS controllers, services, directives, and methods
- **Go to Definition** - Jump to AngularJS component, service, controller, and directive definitions
- **Find References** - Find all usages of AngularJS symbols across your workspace
- **Hover Information** - Display type and documentation information on hover
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
      filetypes = { 'javascript' },
      root_dir = lspconfig.util.root_pattern('package.json', '.git'),
    },
  }
end

lspconfig.angularjs_lsp.setup({})
```

### VS Code

Coming soon.

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
