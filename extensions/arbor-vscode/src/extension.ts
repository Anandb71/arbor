import * as vscode from 'vscode';
import WebSocket from 'ws';
import { execSync, spawn } from 'child_process';

let ws: WebSocket | null = null;
let statusBarItem: vscode.StatusBarItem;
let spotlightDecorationType: vscode.TextEditorDecorationType;
let outputChannel: vscode.OutputChannel;

// Current spotlight state
let currentSpotlightFile: string | null = null;
let currentSpotlightLine: number | null = null;

export function activate(context: vscode.ExtensionContext) {
    console.log('Arbor extension activated');

    // Create output channel for Arbor logs
    outputChannel = vscode.window.createOutputChannel('Arbor');

    // Create status bar item
    statusBarItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
    statusBarItem.text = '$(circuit-board) Arbor';
    statusBarItem.tooltip = 'Arbor Graph Intelligence — Click for commands';
    statusBarItem.command = 'arbor.quickPick';
    statusBarItem.show();

    // Create spotlight decoration (golden glow effect for AI focus)
    spotlightDecorationType = vscode.window.createTextEditorDecorationType({
        backgroundColor: 'rgba(255, 215, 0, 0.15)',
        border: '1px solid rgba(255, 215, 0, 0.5)',
        isWholeLine: true,
        overviewRulerColor: 'gold',
        overviewRulerLane: vscode.OverviewRulerLane.Full,
    });

    // Register all commands
    context.subscriptions.push(
        vscode.commands.registerCommand('arbor.connect', connectToServer),
        vscode.commands.registerCommand('arbor.showInVisualizer', showInVisualizer),
        vscode.commands.registerCommand('arbor.toggleVisualizer', toggleVisualizer),
        vscode.commands.registerCommand('arbor.refactor', analyzeImpact),
        vscode.commands.registerCommand('arbor.status', showStatus),
        vscode.commands.registerCommand('arbor.quickPick', showQuickPick),
        vscode.commands.registerCommand('arbor.diff', showDiffImpact),
        vscode.commands.registerCommand('arbor.index', reindexWorkspace),
    );

    // Auto-connect on startup
    connectToServer();

    context.subscriptions.push(statusBarItem, outputChannel);
}

// ─── Arbor CLI Runner ───────────────────────────────────────────────────────

function getWorkspacePath(): string | undefined {
    const folders = vscode.workspace.workspaceFolders;
    return folders?.[0]?.uri.fsPath;
}

function runArborCommand(args: string[], cwd?: string): string | null {
    const workDir = cwd || getWorkspacePath();
    if (!workDir) {
        vscode.window.showWarningMessage('No workspace folder open');
        return null;
    }

    try {
        const result = execSync(`arbor ${args.join(' ')}`, {
            cwd: workDir,
            encoding: 'utf-8',
            timeout: 60000,
        });
        return result;
    } catch (err: any) {
        const stderr = err.stderr?.toString() || err.message;
        outputChannel.appendLine(`[ERROR] arbor ${args.join(' ')}: ${stderr}`);
        return null;
    }
}

// ─── Impact Analysis (Refactor) ─────────────────────────────────────────────

async function analyzeImpact() {
    const editor = vscode.window.activeTextEditor;

    // Try to get symbol from word under cursor first
    let defaultSymbol = '';
    if (editor) {
        const wordRange = editor.document.getWordRangeAtPosition(editor.selection.active);
        if (wordRange) {
            defaultSymbol = editor.document.getText(wordRange);
        }
    }

    const symbol = await vscode.window.showInputBox({
        prompt: 'Enter symbol name to analyze impact',
        placeHolder: 'e.g., parse_file, AuthController, handleRequest',
        value: defaultSymbol,
    });

    if (!symbol) { return; }

    outputChannel.show(true);
    outputChannel.appendLine(`\n${'═'.repeat(60)}`);
    outputChannel.appendLine(`🔍 Impact Analysis: ${symbol}`);
    outputChannel.appendLine(`${'═'.repeat(60)}\n`);

    const result = runArborCommand(['refactor', symbol, '--why']);

    if (result) {
        outputChannel.appendLine(result);
        vscode.window.showInformationMessage(`Impact analysis complete for "${symbol}"`);
    } else {
        vscode.window.showErrorMessage(
            `Failed to analyze "${symbol}". Is Arbor installed and the workspace indexed?`
        );
        outputChannel.appendLine('Tip: Run "arbor setup" in your terminal first.');
    }
}

// ─── Git Diff Impact ────────────────────────────────────────────────────────

async function showDiffImpact() {
    outputChannel.show(true);
    outputChannel.appendLine(`\n${'═'.repeat(60)}`);
    outputChannel.appendLine('📊 Git Change Impact Preview');
    outputChannel.appendLine(`${'═'.repeat(60)}\n`);

    const result = runArborCommand(['diff']);

    if (result) {
        outputChannel.appendLine(result);
        vscode.window.showInformationMessage('Git diff impact analysis complete');
    } else {
        vscode.window.showWarningMessage(
            'Could not analyze diff. Ensure you have git changes and Arbor is set up.'
        );
    }
}

// ─── Show Index Status ──────────────────────────────────────────────────────

async function showStatus() {
    outputChannel.show(true);
    outputChannel.appendLine(`\n${'═'.repeat(60)}`);
    outputChannel.appendLine('📋 Arbor Index Status');
    outputChannel.appendLine(`${'═'.repeat(60)}\n`);

    const result = runArborCommand(['status']);

    if (result) {
        outputChannel.appendLine(result);
    } else {
        outputChannel.appendLine('Arbor is not initialized in this workspace.');
        outputChannel.appendLine('Run: arbor setup');
    }
}

// ─── Re-index Workspace ─────────────────────────────────────────────────────

async function reindexWorkspace() {
    const workDir = getWorkspacePath();
    if (!workDir) {
        vscode.window.showWarningMessage('No workspace folder open');
        return;
    }

    await vscode.window.withProgress(
        {
            location: vscode.ProgressLocation.Notification,
            title: 'Arbor: Indexing workspace...',
            cancellable: false,
        },
        async () => {
            const result = runArborCommand(['setup']);
            if (result) {
                outputChannel.appendLine(result);
                vscode.window.showInformationMessage('Arbor: Workspace indexed successfully');
            } else {
                vscode.window.showErrorMessage(
                    'Arbor: Indexing failed. Check Output panel for details.'
                );
            }
        }
    );
}

// ─── Quick Pick Command Menu ────────────────────────────────────────────────

async function showQuickPick() {
    const items: vscode.QuickPickItem[] = [
        {
            label: '$(search) Analyze Impact',
            description: 'Predict blast radius before refactoring a symbol',
            detail: 'arbor refactor <symbol>',
        },
        {
            label: '$(git-compare) Git Diff Impact',
            description: 'Preview impact of uncommitted changes',
            detail: 'arbor diff',
        },
        {
            label: '$(info) Show Status',
            description: 'Display index stats and graph health',
            detail: 'arbor status',
        },
        {
            label: '$(refresh) Re-index Workspace',
            description: 'Rebuild the code graph from scratch',
            detail: 'arbor setup',
        },
        {
            label: '$(eye) Show in Visualizer',
            description: 'Focus current symbol in the graph visualizer',
        },
        {
            label: '$(graph) Toggle Visualizer',
            description: 'Launch the Arbor graph visualizer',
        },
        {
            label: '$(plug) Connect to Server',
            description: 'Connect to the Arbor WebSocket server',
        },
    ];

    const selected = await vscode.window.showQuickPick(items, {
        placeHolder: 'Select an Arbor command...',
        matchOnDescription: true,
        matchOnDetail: true,
    });

    if (!selected) { return; }

    switch (selected.label) {
        case '$(search) Analyze Impact':
            vscode.commands.executeCommand('arbor.refactor');
            break;
        case '$(git-compare) Git Diff Impact':
            vscode.commands.executeCommand('arbor.diff');
            break;
        case '$(info) Show Status':
            vscode.commands.executeCommand('arbor.status');
            break;
        case '$(refresh) Re-index Workspace':
            vscode.commands.executeCommand('arbor.index');
            break;
        case '$(eye) Show in Visualizer':
            vscode.commands.executeCommand('arbor.showInVisualizer');
            break;
        case '$(graph) Toggle Visualizer':
            vscode.commands.executeCommand('arbor.toggleVisualizer');
            break;
        case '$(plug) Connect to Server':
            vscode.commands.executeCommand('arbor.connect');
            break;
    }
}

// ─── WebSocket Connection ───────────────────────────────────────────────────

function connectToServer() {
    const config = vscode.workspace.getConfiguration('arbor');
    const serverUrl = config.get<string>('serverUrl', 'ws://127.0.0.1:8080');

    if (ws && ws.readyState === WebSocket.OPEN) {
        updateStatusBar('connected');
        return;
    }

    updateStatusBar('connecting');

    try {
        ws = new WebSocket(serverUrl);

        ws.on('open', () => {
            updateStatusBar('connected');
            outputChannel.appendLine('[Arbor] Connected to server');
        });

        ws.on('message', (data: Buffer) => {
            try {
                const message = JSON.parse(data.toString());
                handleServerMessage(message);
            } catch (e) {
                console.error('Failed to parse Arbor message:', e);
            }
        });

        ws.on('close', () => {
            updateStatusBar('disconnected');
            ws = null;
            clearSpotlight();
            outputChannel.appendLine('[Arbor] Disconnected from server');
        });

        ws.on('error', (err) => {
            // Silently handle connection errors (server may not be running)
            updateStatusBar('disconnected');
            console.error('Arbor WebSocket error:', err.message);
        });
    } catch (err: any) {
        updateStatusBar('disconnected');
    }
}

function updateStatusBar(state: 'connected' | 'connecting' | 'disconnected') {
    switch (state) {
        case 'connected':
            statusBarItem.text = '$(circuit-board) Arbor';
            statusBarItem.tooltip = 'Arbor Graph Intelligence — Connected';
            statusBarItem.backgroundColor = undefined;
            break;
        case 'connecting':
            statusBarItem.text = '$(sync~spin) Arbor';
            statusBarItem.tooltip = 'Connecting to Arbor server...';
            break;
        case 'disconnected':
            statusBarItem.text = '$(circuit-board) Arbor';
            statusBarItem.tooltip = 'Arbor Graph Intelligence — Click for commands';
            statusBarItem.backgroundColor = undefined;
            break;
    }
}

// ─── Server Message Handling ────────────────────────────────────────────────

function handleServerMessage(message: any) {
    if (message.type === 'FocusNode') {
        const payload = message.payload;
        if (payload.file && payload.line !== undefined) {
            highlightSpotlight(payload.file, payload.line);
        }
    }
}

async function highlightSpotlight(filePath: string, line: number) {
    currentSpotlightFile = filePath;
    currentSpotlightLine = line;

    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders) { return; }

    const files = await vscode.workspace.findFiles(`**/${filePath.split(/[\\/]/).pop()}`);
    if (files.length === 0) { return; }

    const document = await vscode.workspace.openTextDocument(files[0]);
    const editor = await vscode.window.showTextDocument(document, { preview: true });

    const lineIndex = Math.max(0, line - 1);
    const range = new vscode.Range(lineIndex, 0, lineIndex, Number.MAX_VALUE);

    editor.setDecorations(spotlightDecorationType, [range]);
    editor.revealRange(range, vscode.TextEditorRevealType.InCenter);

    setTimeout(() => {
        if (currentSpotlightLine === line && currentSpotlightFile === filePath) {
            clearSpotlight();
        }
    }, 3000);
}

function clearSpotlight() {
    currentSpotlightFile = null;
    currentSpotlightLine = null;
    vscode.window.visibleTextEditors.forEach(editor => {
        editor.setDecorations(spotlightDecorationType, []);
    });
}

// ─── Visualizer Commands ────────────────────────────────────────────────────

async function showInVisualizer() {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
        vscode.window.showWarningMessage('No active editor');
        return;
    }

    const document = editor.document;
    const position = editor.selection.active;
    const line = position.line + 1;
    const fileName = document.fileName;

    if (ws && ws.readyState === WebSocket.OPEN) {
        const request = {
            jsonrpc: '2.0',
            method: 'spotlight.focus',
            params: { file: fileName, line },
            id: Date.now(),
        };
        ws.send(JSON.stringify(request));
        vscode.window.showInformationMessage(`Showing ${fileName}:${line} in Arbor`);
    } else {
        vscode.window.showWarningMessage('Not connected to Arbor server. Use the command palette to connect.');
    }
}

async function toggleVisualizer() {
    const workspaceFolders = vscode.workspace.workspaceFolders;
    if (!workspaceFolders || workspaceFolders.length === 0) {
        vscode.window.showWarningMessage('No workspace folder open');
        return;
    }

    const terminal = vscode.window.createTerminal({
        name: 'Arbor Visualizer',
        cwd: workspaceFolders[0].uri.fsPath,
    });
    terminal.show();
    terminal.sendText('arbor viz');
    vscode.window.showInformationMessage('Launching Arbor Visualizer...');
}

// ─── Deactivation ───────────────────────────────────────────────────────────

export function deactivate() {
    if (ws) {
        ws.close();
    }
}
