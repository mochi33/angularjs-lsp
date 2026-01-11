import * as vscode from 'vscode';
import * as path from 'path';
import * as fs from 'fs';
import * as https from 'https';
import * as http from 'http';
import { exec } from 'child_process';
import { promisify } from 'util';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

const execAsync = promisify(exec);

let client: LanguageClient | undefined;

interface LspLocation {
    uri: string;
    range: {
        start: { line: number; character: number };
        end: { line: number; character: number };
    };
}

// GitHub release info
const GITHUB_OWNER = 'mochi33';
const GITHUB_REPO = 'angularjs-lsp';
const LSP_VERSION = '0.1.0';

export async function activate(context: vscode.ExtensionContext) {
    const outputChannel = vscode.window.createOutputChannel('AngularJS LSP');
    context.subscriptions.push(outputChannel);

    const openLocationDisposable = vscode.commands.registerCommand(
        'angularjs.openLocation',
        async (locations: LspLocation[]) => {
            await handleOpenLocation(locations, outputChannel);
        }
    );
    context.subscriptions.push(openLocationDisposable);

    const restartDisposable = vscode.commands.registerCommand(
        'angularjs.restartServer',
        async () => {
            await restartServer(context, outputChannel);
        }
    );
    context.subscriptions.push(restartDisposable);

    // Ensure dependencies are installed
    await ensureDependencies(context, outputChannel);

    await startServer(context, outputChannel);

    context.subscriptions.push(
        vscode.workspace.onDidChangeConfiguration(async (e) => {
            if (e.affectsConfiguration('angularjsLsp.serverPath')) {
                const result = await vscode.window.showInformationMessage(
                    'Server path changed. Restart the language server?',
                    'Yes',
                    'No'
                );
                if (result === 'Yes') {
                    await restartServer(context, outputChannel);
                }
            }
        })
    );
}

/**
 * Ensure all dependencies are installed
 */
async function ensureDependencies(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    // Check and install typescript-language-server
    await ensureTypescriptLanguageServer(outputChannel);

    // Check and install angularjs-lsp
    await ensureAngularJsLsp(context, outputChannel);
}

/**
 * Check if typescript-language-server is installed, install if not
 */
async function ensureTypescriptLanguageServer(
    outputChannel: vscode.OutputChannel
): Promise<void> {
    const tsLspPath = await findExecutable('typescript-language-server');

    if (tsLspPath) {
        outputChannel.appendLine(`typescript-language-server found: ${tsLspPath}`);
        return;
    }

    outputChannel.appendLine('typescript-language-server not found, installing...');

    const result = await vscode.window.showInformationMessage(
        'typescript-language-server is not installed. Install it globally via npm?',
        'Install',
        'Skip'
    );

    if (result !== 'Install') {
        outputChannel.appendLine('Skipped typescript-language-server installation');
        return;
    }

    try {
        await vscode.window.withProgress(
            {
                location: vscode.ProgressLocation.Notification,
                title: 'Installing typescript-language-server...',
                cancellable: false,
            },
            async () => {
                await execAsync('npm install -g typescript-language-server typescript');
            }
        );
        outputChannel.appendLine('typescript-language-server installed successfully');
        vscode.window.showInformationMessage('typescript-language-server installed successfully');
    } catch (error) {
        outputChannel.appendLine(`Failed to install typescript-language-server: ${error}`);
        vscode.window.showErrorMessage(
            `Failed to install typescript-language-server: ${error}. Please install manually: npm install -g typescript-language-server typescript`
        );
    }
}

/**
 * Check if angularjs-lsp is available, download if not
 */
async function ensureAngularJsLsp(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    const config = vscode.workspace.getConfiguration('angularjsLsp');
    let serverPath = config.get<string>('serverPath');

    // If serverPath is set and exists, use it
    if (serverPath && await fileExists(serverPath)) {
        outputChannel.appendLine(`angularjs-lsp found at configured path: ${serverPath}`);
        return;
    }

    // Check in extension's bin directory
    const binDir = path.join(context.globalStorageUri.fsPath, 'bin');
    const executableName = getExecutableName();
    const localServerPath = path.join(binDir, executableName);

    if (await fileExists(localServerPath)) {
        outputChannel.appendLine(`angularjs-lsp found at: ${localServerPath}`);
        // Update config to use local path
        await config.update('serverPath', localServerPath, vscode.ConfigurationTarget.Global);
        return;
    }

    // Check if it's in PATH
    const pathServerPath = await findExecutable('angularjs-lsp');
    if (pathServerPath) {
        outputChannel.appendLine(`angularjs-lsp found in PATH: ${pathServerPath}`);
        await config.update('serverPath', pathServerPath, vscode.ConfigurationTarget.Global);
        return;
    }

    // Not found, ask to download
    outputChannel.appendLine('angularjs-lsp not found');

    const result = await vscode.window.showInformationMessage(
        'angularjs-lsp is not installed. Download it from GitHub releases?',
        'Download',
        'Skip'
    );

    if (result !== 'Download') {
        outputChannel.appendLine('Skipped angularjs-lsp download');
        return;
    }

    try {
        await downloadAngularJsLsp(context, outputChannel);
        await config.update('serverPath', localServerPath, vscode.ConfigurationTarget.Global);
    } catch (error) {
        outputChannel.appendLine(`Failed to download angularjs-lsp: ${error}`);
        vscode.window.showErrorMessage(
            `Failed to download angularjs-lsp: ${error}. Please download manually from GitHub releases.`
        );
    }
}

/**
 * Download angularjs-lsp from GitHub releases
 */
async function downloadAngularJsLsp(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    const binDir = path.join(context.globalStorageUri.fsPath, 'bin');
    const executableName = getExecutableName();
    const localServerPath = path.join(binDir, executableName);

    // Create bin directory
    await fs.promises.mkdir(binDir, { recursive: true });

    const assetName = getAssetName();
    const downloadUrl = `https://github.com/${GITHUB_OWNER}/${GITHUB_REPO}/releases/download/v${LSP_VERSION}/${assetName}`;

    outputChannel.appendLine(`Downloading angularjs-lsp from: ${downloadUrl}`);

    await vscode.window.withProgress(
        {
            location: vscode.ProgressLocation.Notification,
            title: 'Downloading angularjs-lsp...',
            cancellable: false,
        },
        async (progress) => {
            await downloadFile(downloadUrl, localServerPath, (percent) => {
                progress.report({ message: `${percent}%` });
            });

            // Make executable on Unix-like systems
            if (process.platform !== 'win32') {
                await fs.promises.chmod(localServerPath, 0o755);
            }
        }
    );

    outputChannel.appendLine(`angularjs-lsp downloaded to: ${localServerPath}`);
    vscode.window.showInformationMessage('angularjs-lsp downloaded successfully');
}

/**
 * Get the executable name based on platform
 */
function getExecutableName(): string {
    if (process.platform === 'win32') {
        return 'angularjs-lsp.exe';
    }
    return 'angularjs-lsp';
}

/**
 * Get the asset name for GitHub release based on platform and architecture
 */
function getAssetName(): string {
    const platform = process.platform;
    const arch = process.arch;

    if (platform === 'win32') {
        return arch === 'x64' ? 'angularjs-lsp-x86_64-pc-windows-msvc.exe' : 'angularjs-lsp-i686-pc-windows-msvc.exe';
    } else if (platform === 'darwin') {
        return arch === 'arm64' ? 'angularjs-lsp-aarch64-apple-darwin' : 'angularjs-lsp-x86_64-apple-darwin';
    } else {
        // Linux
        return arch === 'arm64' ? 'angularjs-lsp-aarch64-unknown-linux-gnu' : 'angularjs-lsp-x86_64-unknown-linux-gnu';
    }
}

/**
 * Download a file from URL
 */
async function downloadFile(
    url: string,
    destPath: string,
    onProgress?: (percent: number) => void
): Promise<void> {
    return new Promise((resolve, reject) => {
        const request = (url: string) => {
            const protocol = url.startsWith('https') ? https : http;

            protocol.get(url, { headers: { 'User-Agent': 'VSCode-AngularJS-LSP' } }, (response) => {
                // Handle redirects
                if (response.statusCode === 301 || response.statusCode === 302) {
                    const redirectUrl = response.headers.location;
                    if (redirectUrl) {
                        request(redirectUrl);
                        return;
                    }
                }

                if (response.statusCode !== 200) {
                    reject(new Error(`Failed to download: HTTP ${response.statusCode}`));
                    return;
                }

                const totalSize = parseInt(response.headers['content-length'] || '0', 10);
                let downloadedSize = 0;

                const fileStream = fs.createWriteStream(destPath);

                response.on('data', (chunk: Buffer) => {
                    downloadedSize += chunk.length;
                    if (totalSize > 0 && onProgress) {
                        onProgress(Math.round((downloadedSize / totalSize) * 100));
                    }
                });

                response.pipe(fileStream);

                fileStream.on('finish', () => {
                    fileStream.close();
                    resolve();
                });

                fileStream.on('error', (err) => {
                    fs.unlink(destPath, () => {});
                    reject(err);
                });
            }).on('error', reject);
        };

        request(url);
    });
}

/**
 * Find executable in PATH
 */
async function findExecutable(name: string): Promise<string | null> {
    const command = process.platform === 'win32' ? 'where' : 'which';

    try {
        const { stdout } = await execAsync(`${command} ${name}`);
        const result = stdout.trim().split('\n')[0];
        return result || null;
    } catch {
        return null;
    }
}

/**
 * Check if file exists
 */
async function fileExists(filePath: string): Promise<boolean> {
    try {
        await fs.promises.access(filePath);
        return true;
    } catch {
        return false;
    }
}

async function startServer(
    _context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    const config = vscode.workspace.getConfiguration('angularjsLsp');
    const serverPath = config.get<string>('serverPath');

    if (!serverPath) {
        outputChannel.appendLine(
            'AngularJS LSP: Server path not configured. Please set "angularjsLsp.serverPath" in settings.'
        );
        vscode.window.showWarningMessage(
            'AngularJS LSP: Server path not configured. Please set "angularjsLsp.serverPath" in settings.'
        );
        return;
    }

    try {
        await vscode.workspace.fs.stat(vscode.Uri.file(serverPath));
    } catch {
        outputChannel.appendLine(
            `AngularJS LSP: Server executable not found at: ${serverPath}`
        );
        vscode.window.showErrorMessage(
            `AngularJS LSP: Server executable not found at: ${serverPath}`
        );
        return;
    }

    outputChannel.appendLine(`AngularJS LSP: Starting server from ${serverPath}`);

    const serverOptions: ServerOptions = {
        run: {
            command: serverPath,
            transport: TransportKind.stdio,
        },
        debug: {
            command: serverPath,
            transport: TransportKind.stdio,
            options: {
                env: {
                    ...process.env,
                    RUST_LOG: 'debug',
                },
            },
        },
    };

    const workspaceFolder = vscode.workspace.workspaceFolders?.[0];

    const clientOptions: LanguageClientOptions = {
        documentSelector: [
            { scheme: 'file', language: 'javascript' },
            { scheme: 'file', language: 'html' },
        ],
        synchronize: {
            fileEvents: vscode.workspace.createFileSystemWatcher('**/ajsconfig.json'),
        },
        outputChannel: outputChannel,
        workspaceFolder: workspaceFolder,
    };

    client = new LanguageClient(
        'angularjsLsp',
        'AngularJS Language Server',
        serverOptions,
        clientOptions
    );

    try {
        await client.start();
        outputChannel.appendLine('AngularJS LSP: Server started successfully');
    } catch (error) {
        outputChannel.appendLine(`AngularJS LSP: Failed to start server: ${error}`);
        vscode.window.showErrorMessage(`AngularJS LSP: Failed to start server: ${error}`);
        client = undefined;
    }
}

async function stopServer(): Promise<void> {
    if (client) {
        try {
            // Only stop if the client is actually running
            // isRunning() returns false for startFailed state
            if (client.isRunning()) {
                await client.stop();
            } else {
                // For non-running states, just dispose
                client.dispose();
            }
        } catch {
            // Ignore all errors when stopping
        } finally {
            client = undefined;
        }
    }
}

async function restartServer(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    outputChannel.appendLine('AngularJS LSP: Restarting server...');
    await stopServer();
    await startServer(context, outputChannel);
}

async function handleOpenLocation(
    locations: LspLocation[],
    outputChannel: vscode.OutputChannel
): Promise<void> {
    if (!locations || locations.length === 0) {
        outputChannel.appendLine('AngularJS LSP: openLocation called with no locations');
        return;
    }

    if (locations.length === 1) {
        await openSingleLocation(locations[0]);
    } else {
        const items = locations.map((loc) => {
            const uri = vscode.Uri.parse(loc.uri);
            const filename = path.basename(uri.fsPath);
            const line = loc.range.start.line + 1;
            return {
                label: filename,
                description: `Line ${line}`,
                detail: uri.fsPath,
                location: loc,
            };
        });

        const selected = await vscode.window.showQuickPick(items, {
            placeHolder: 'Select a location to open',
        });

        if (selected) {
            await openSingleLocation(selected.location);
        }
    }
}

async function openSingleLocation(location: LspLocation): Promise<void> {
    const uri = vscode.Uri.parse(location.uri);
    const range = new vscode.Range(
        new vscode.Position(location.range.start.line, location.range.start.character),
        new vscode.Position(location.range.end.line, location.range.end.character)
    );

    const document = await vscode.workspace.openTextDocument(uri);
    const editor = await vscode.window.showTextDocument(document);

    editor.selection = new vscode.Selection(range.start, range.start);
    editor.revealRange(range, vscode.TextEditorRevealType.InCenter);
}

export async function deactivate(): Promise<void> {
    await stopServer();
}
