import * as vscode from 'vscode';
import * as path from 'path';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
    TransportKind,
} from 'vscode-languageclient/node';

let client: LanguageClient | undefined;

interface LspLocation {
    uri: string;
    range: {
        start: { line: number; character: number };
        end: { line: number; character: number };
    };
}

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
