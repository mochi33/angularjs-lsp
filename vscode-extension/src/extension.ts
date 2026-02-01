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

/**
 * Safely register a command, checking if it already exists first.
 * This prevents "command already exists" errors when vscode-languageclient
 * auto-registers commands from the server's executeCommandProvider.
 */
async function safeRegisterCommand(
    commandId: string,
    callback: (...args: any[]) => any
): Promise<vscode.Disposable | undefined> {
    const existingCommands = await vscode.commands.getCommands(true);
    if (existingCommands.includes(commandId)) {
        console.warn(`Command ${commandId} already registered, skipping`);
        return undefined;
    }
    return vscode.commands.registerCommand(commandId, callback);
}

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
const CURRENT_VERSION = '0.1.5';

// State keys
const STATE_INSTALLED_VERSION = 'angularjsLsp.installedVersion';
const STATE_LAST_UPDATE_CHECK = 'angularjsLsp.lastUpdateCheck';
const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60 * 1000; // 24 hours

interface GitHubRelease {
    tag_name: string;
    name: string;
    html_url: string;
    assets: Array<{
        name: string;
        browser_download_url: string;
    }>;
}

export async function activate(context: vscode.ExtensionContext) {
    const outputChannel = vscode.window.createOutputChannel('AngularJS LSP');
    context.subscriptions.push(outputChannel);

    // Register commands with safe registration to avoid "command already exists" errors
    // when vscode-languageclient auto-registers commands from the server
    const openLocationDisposable = await safeRegisterCommand(
        'angularjs.openLocation',
        async (locations: LspLocation[]) => {
            await handleOpenLocation(locations, outputChannel);
        }
    );
    if (openLocationDisposable) {
        context.subscriptions.push(openLocationDisposable);
    }

    const restartDisposable = await safeRegisterCommand(
        'angularjs.restartServer',
        async () => {
            await restartServer(context, outputChannel);
        }
    );
    if (restartDisposable) {
        context.subscriptions.push(restartDisposable);
    }

    const installServerDisposable = await safeRegisterCommand(
        'angularjs.installServer',
        async () => {
            await installServerCommand(context, outputChannel);
        }
    );
    if (installServerDisposable) {
        context.subscriptions.push(installServerDisposable);
    }

    const refreshIndexDisposable = await safeRegisterCommand(
        'angularjs.refreshIndex',
        async () => {
            await refreshIndexCommand(outputChannel);
        }
    );
    if (refreshIndexDisposable) {
        context.subscriptions.push(refreshIndexDisposable);
    }

    const refreshCacheDisposable = await safeRegisterCommand(
        'angularjs.refreshCache',
        async () => {
            await refreshCacheCommand(outputChannel);
        }
    );
    if (refreshCacheDisposable) {
        context.subscriptions.push(refreshCacheDisposable);
    }

    // Ensure dependencies are installed (non-blocking for dialogs)
    await ensureDependencies(context, outputChannel);

    // Start server first, then check for updates in background
    await startServer(context, outputChannel);

    // Check for updates (non-blocking)
    checkForUpdates(context, outputChannel).catch((err) => {
        outputChannel.appendLine(`Update check failed: ${err}`);
    });

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
    // Check and install angularjs-lsp (required)
    await ensureAngularJsLsp(context, outputChannel);

    // Check typescript-language-server in background (optional, for fallback)
    ensureTypescriptLanguageServer(outputChannel).catch((err) => {
        outputChannel.appendLine(`typescript-language-server check failed: ${err}`);
    });
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

    // Fetch latest release from GitHub API
    const latestRelease = await fetchLatestRelease();
    if (!latestRelease) {
        throw new Error('Could not fetch latest release from GitHub');
    }

    const assetName = getAssetName();
    const asset = latestRelease.assets.find((a) => a.name === assetName);
    if (!asset) {
        throw new Error(`No binary available for your platform (${assetName})`);
    }

    const downloadUrl = asset.browser_download_url;
    const version = latestRelease.tag_name.replace(/^v/, '');

    outputChannel.appendLine(`Downloading angularjs-lsp v${version} from: ${downloadUrl}`);

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

    // Save installed version
    await context.globalState.update(STATE_INSTALLED_VERSION, version);

    outputChannel.appendLine(`angularjs-lsp v${version} downloaded to: ${localServerPath}`);
    vscode.window.showInformationMessage(`angularjs-lsp v${version} downloaded successfully`);
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
    context: vscode.ExtensionContext,
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

    // Find typescript-language-server and add its directory to PATH
    const tsLspPath = await findExecutable('typescript-language-server');
    let envPath = process.env.PATH || '';
    if (tsLspPath) {
        const tsLspDir = path.dirname(tsLspPath);
        outputChannel.appendLine(`Adding typescript-language-server directory to PATH: ${tsLspDir}`);
        envPath = `${tsLspDir}${path.delimiter}${envPath}`;
    }

    const serverOptions: ServerOptions = {
        run: {
            command: serverPath,
            transport: TransportKind.stdio,
            options: {
                env: {
                    ...process.env,
                    PATH: envPath,
                },
            },
        },
        debug: {
            command: serverPath,
            transport: TransportKind.stdio,
            options: {
                env: {
                    ...process.env,
                    PATH: envPath,
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
            { scheme: 'file', language: 'django-html' },
            { scheme: 'file', language: 'jinja-html' },
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
        // Add client to subscriptions for proper disposal on extension deactivate/reload
        context.subscriptions.push(client);
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

/**
 * Command to refresh the AngularJS index
 */
async function refreshIndexCommand(
    outputChannel: vscode.OutputChannel
): Promise<void> {
    if (!client) {
        vscode.window.showWarningMessage('AngularJS LSP: Language server is not running');
        return;
    }

    outputChannel.appendLine('AngularJS LSP: Refreshing index...');

    try {
        await vscode.window.withProgress(
            {
                location: vscode.ProgressLocation.Notification,
                title: 'Refreshing AngularJS index...',
                cancellable: false,
            },
            async () => {
                await client!.sendRequest('workspace/executeCommand', {
                    command: 'angularjs-lsp.refreshIndex',
                });
            }
        );

        outputChannel.appendLine('AngularJS LSP: Index refreshed');
        vscode.window.showInformationMessage('AngularJS index refreshed');
    } catch (error) {
        outputChannel.appendLine(`AngularJS LSP: Failed to refresh index: ${error}`);
        vscode.window.showErrorMessage(`Failed to refresh index: ${error}`);
    }
}

/**
 * Command to refresh the AngularJS cache (re-index and save cache)
 */
async function refreshCacheCommand(
    outputChannel: vscode.OutputChannel
): Promise<void> {
    if (!client) {
        vscode.window.showWarningMessage('AngularJS LSP: Language server is not running');
        return;
    }

    outputChannel.appendLine('AngularJS LSP: Refreshing cache...');

    try {
        await vscode.window.withProgress(
            {
                location: vscode.ProgressLocation.Notification,
                title: 'Refreshing AngularJS cache...',
                cancellable: false,
            },
            async () => {
                // Use existing refreshIndex command which will re-scan and save cache
                await client!.sendRequest('workspace/executeCommand', {
                    command: 'angularjs-lsp.refreshIndex',
                });
            }
        );

        outputChannel.appendLine('AngularJS LSP: Cache refreshed');
        vscode.window.showInformationMessage('AngularJS cache refreshed');
    } catch (error) {
        outputChannel.appendLine(`AngularJS LSP: Failed to refresh cache: ${error}`);
        vscode.window.showErrorMessage(`Failed to refresh cache: ${error}`);
    }
}

export async function deactivate(): Promise<void> {
    await stopServer();
}

/**
 * Check for updates from GitHub releases
 */
async function checkForUpdates(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    // Check if enough time has passed since last check
    const lastCheck = context.globalState.get<number>(STATE_LAST_UPDATE_CHECK) || 0;
    const now = Date.now();

    if (now - lastCheck < UPDATE_CHECK_INTERVAL_MS) {
        outputChannel.appendLine('Skipping update check (checked recently)');
        return;
    }

    outputChannel.appendLine('Checking for angularjs-lsp updates...');

    try {
        const latestRelease = await fetchLatestRelease();
        if (!latestRelease) {
            outputChannel.appendLine('Could not fetch latest release info');
            return;
        }

        // Update last check time
        await context.globalState.update(STATE_LAST_UPDATE_CHECK, now);

        const latestVersion = latestRelease.tag_name.replace(/^v/, '');
        const installedVersion = context.globalState.get<string>(STATE_INSTALLED_VERSION) || CURRENT_VERSION;

        outputChannel.appendLine(`Installed version: ${installedVersion}, Latest version: ${latestVersion}`);

        if (compareVersions(latestVersion, installedVersion) > 0) {
            // New version available
            const result = await vscode.window.showInformationMessage(
                `A new version of angularjs-lsp is available: v${latestVersion} (current: v${installedVersion})`,
                'Update Now',
                'View Release',
                'Later'
            );

            if (result === 'Update Now') {
                await performUpdate(context, outputChannel, latestRelease, latestVersion);
            } else if (result === 'View Release') {
                vscode.env.openExternal(vscode.Uri.parse(latestRelease.html_url));
            }
        } else {
            outputChannel.appendLine('angularjs-lsp is up to date');
        }
    } catch (error) {
        outputChannel.appendLine(`Update check error: ${error}`);
    }
}

/**
 * Fetch the latest release from GitHub API
 */
async function fetchLatestRelease(): Promise<GitHubRelease | null> {
    return new Promise((resolve) => {
        const options = {
            hostname: 'api.github.com',
            path: `/repos/${GITHUB_OWNER}/${GITHUB_REPO}/releases/latest`,
            headers: {
                'User-Agent': 'VSCode-AngularJS-LSP',
                'Accept': 'application/vnd.github.v3+json',
            },
        };

        https.get(options, (response) => {
            if (response.statusCode === 404) {
                // No releases yet
                resolve(null);
                return;
            }

            if (response.statusCode !== 200) {
                resolve(null);
                return;
            }

            let data = '';
            response.on('data', (chunk) => {
                data += chunk;
            });

            response.on('end', () => {
                try {
                    const release = JSON.parse(data) as GitHubRelease;
                    resolve(release);
                } catch {
                    resolve(null);
                }
            });
        }).on('error', () => {
            resolve(null);
        });
    });
}

/**
 * Compare two semantic versions
 * Returns: positive if v1 > v2, negative if v1 < v2, 0 if equal
 */
function compareVersions(v1: string, v2: string): number {
    const parts1 = v1.split('.').map(Number);
    const parts2 = v2.split('.').map(Number);

    for (let i = 0; i < Math.max(parts1.length, parts2.length); i++) {
        const p1 = parts1[i] || 0;
        const p2 = parts2[i] || 0;
        if (p1 !== p2) {
            return p1 - p2;
        }
    }

    return 0;
}

/**
 * Perform the update
 */
async function performUpdate(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel,
    release: GitHubRelease,
    version: string
): Promise<void> {
    const binDir = path.join(context.globalStorageUri.fsPath, 'bin');
    const executableName = getExecutableName();
    const localServerPath = path.join(binDir, executableName);

    // Find the correct asset for this platform
    const assetName = getAssetName();
    const asset = release.assets.find((a) => a.name === assetName);

    if (!asset) {
        vscode.window.showErrorMessage(
            `No binary available for your platform (${assetName}). Please download manually.`
        );
        return;
    }

    outputChannel.appendLine(`Updating angularjs-lsp to v${version}...`);

    try {
        // Stop the current server
        await stopServer();

        // Create bin directory if needed
        await fs.promises.mkdir(binDir, { recursive: true });

        // Download new version
        await vscode.window.withProgress(
            {
                location: vscode.ProgressLocation.Notification,
                title: `Updating angularjs-lsp to v${version}...`,
                cancellable: false,
            },
            async (progress) => {
                await downloadFile(asset.browser_download_url, localServerPath, (percent) => {
                    progress.report({ message: `${percent}%` });
                });

                // Make executable on Unix-like systems
                if (process.platform !== 'win32') {
                    await fs.promises.chmod(localServerPath, 0o755);
                }
            }
        );

        // Update stored version
        await context.globalState.update(STATE_INSTALLED_VERSION, version);

        // Update config if needed
        const config = vscode.workspace.getConfiguration('angularjsLsp');
        await config.update('serverPath', localServerPath, vscode.ConfigurationTarget.Global);

        outputChannel.appendLine(`angularjs-lsp updated to v${version}`);

        // Offer to restart
        const result = await vscode.window.showInformationMessage(
            `angularjs-lsp has been updated to v${version}. Restart the language server?`,
            'Restart',
            'Later'
        );

        if (result === 'Restart') {
            await startServer(context, outputChannel);
        }
    } catch (error) {
        outputChannel.appendLine(`Update failed: ${error}`);
        vscode.window.showErrorMessage(`Failed to update angularjs-lsp: ${error}`);
    }
}

/**
 * Command to manually install/update angularjs-lsp server
 */
async function installServerCommand(
    context: vscode.ExtensionContext,
    outputChannel: vscode.OutputChannel
): Promise<void> {
    outputChannel.show();
    outputChannel.appendLine('Installing angularjs-lsp server...');

    try {
        // Stop current server if running
        await stopServer();

        // Wait for server process to fully terminate
        await new Promise((resolve) => setTimeout(resolve, 1000));

        // Delete existing binary to avoid ETXTBSY
        const binDir = path.join(context.globalStorageUri.fsPath, 'bin');
        const executableName = getExecutableName();
        const localServerPath = path.join(binDir, executableName);
        try {
            await fs.promises.unlink(localServerPath);
            outputChannel.appendLine('Removed existing binary');
        } catch {
            // File might not exist, ignore
        }

        // Download latest version
        await downloadAngularJsLsp(context, outputChannel);

        // Update config
        const config = vscode.workspace.getConfiguration('angularjsLsp');
        await config.update('serverPath', localServerPath, vscode.ConfigurationTarget.Global);

        // Restart server
        const result = await vscode.window.showInformationMessage(
            'angularjs-lsp installed successfully. Start the language server?',
            'Start',
            'Later'
        );

        if (result === 'Start') {
            await startServer(context, outputChannel);
        }
    } catch (error) {
        outputChannel.appendLine(`Installation failed: ${error}`);
        vscode.window.showErrorMessage(`Failed to install angularjs-lsp: ${error}`);
    }
}
