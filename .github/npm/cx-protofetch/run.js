#!/usr/bin/env node
import { spawn } from 'node:child_process';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const isWindows = process.platform === 'win32';
const binaryName = isWindows ? 'protofetch.exe' : 'protofetch';
const binaryPath = path.join(__dirname, 'bin', binaryName);

const child = spawn(binaryPath, process.argv.slice(2), { stdio: 'inherit' });

child.on('error', (error) => {
	console.error(`Failed to start protofetch: ${error.message}`);
	console.error('The binary may be missing or corrupted. Try reinstalling the package:');
	console.error('  npm install --force');
	console.error('  or');
	console.error('  pnpm install --force');
	process.exit(1);
});

child.on('close', (code) => {
	process.exit(code);
});
