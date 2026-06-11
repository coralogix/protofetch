#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import path from 'node:path';

const require = createRequire(import.meta.url);

// Maps the host (`${process.platform} ${process.arch}`) to the platform package
// that ships its prebuilt binary, plus the binary filename inside that package.
// This is the inverse of the rust target table in prepare-package.js and must
// stay in sync with it.
//
// Note: the two linux entries are served by static musl binaries, which run on
// both glibc and musl hosts, so there is a single linux package per arch (no
// gnu/musl split and no `libc` field on the platform packages).
const PLATFORM_PACKAGES = {
	'darwin arm64': ['@coralogix/protofetch-darwin-arm64', 'protofetch'],
	'darwin x64': ['@coralogix/protofetch-darwin-x64', 'protofetch'],
	'linux arm64': ['@coralogix/protofetch-linux-arm64', 'protofetch'],
	'linux x64': ['@coralogix/protofetch-linux-x64', 'protofetch'],
	'win32 x64': ['@coralogix/protofetch-win32-x64', 'protofetch.exe']
};

function resolveBinaryPath() {
	// 1. Explicit override for CI, debugging, or unusual install layouts.
	if (process.env.PROTOFETCH_BINARY_PATH) {
		return process.env.PROTOFETCH_BINARY_PATH;
	}

	const hostKey = `${process.platform} ${process.arch}`;
	const entry = PLATFORM_PACKAGES[hostKey];
	if (!entry) {
		failUnsupportedPlatform(hostKey);
	}

	const [pkg, binaryName] = entry;

	// 2. Resolve the platform package via its package.json (layout-agnostic
	//    across npm hoisting and pnpm's symlinked store), then join the binary
	//    path. Resolving package.json rather than the binary file directly
	//    avoids brittleness with non-JS files and `exports` maps.
	try {
		const pkgJsonPath = require.resolve(`${pkg}/package.json`);
		return path.join(path.dirname(pkgJsonPath), 'bin', binaryName);
	} catch {
		failMissingPackage(pkg, hostKey);
	}
}

function failUnsupportedPlatform(hostKey) {
	console.error(`protofetch: unsupported platform "${hostKey}".`);
	console.error('Supported platforms: ' + Object.keys(PLATFORM_PACKAGES).join(', ') + '.');
	console.error('Please open an issue at https://github.com/coralogix/protofetch/issues');
	process.exit(1);
}

function failMissingPackage(pkg, hostKey) {
	console.error(`protofetch: could not find the prebuilt binary package "${pkg}" for ${hostKey}.`);
	console.error('');
	console.error('This usually happens when optional dependencies were skipped, e.g.');
	console.error('installing with --no-optional or --omit=optional, or a corporate proxy');
	console.error('that strips optional dependencies. Reinstall without that option:');
	console.error('  npm install @coralogix/protofetch');
	console.error('');
	console.error('Alternatively, point protofetch at a binary you provide yourself:');
	console.error('  PROTOFETCH_BINARY_PATH=/path/to/protofetch protofetch ...');
	console.error('');
	console.error('If the problem persists, open an issue at https://github.com/coralogix/protofetch/issues');
	process.exit(1);
}

const binaryPath = resolveBinaryPath();

const child = spawn(binaryPath, process.argv.slice(2), { stdio: 'inherit' });

child.on('error', (error) => {
	console.error(`Failed to start protofetch: ${error.message}`);
	console.error(`Tried to execute: ${binaryPath}`);
	console.error('The binary may be missing or corrupted. Try reinstalling the package:');
	console.error('  npm install --force @coralogix/protofetch');
	process.exit(1);
});

child.on('close', (code, signal) => {
	if (signal) {
		// The binary was killed by a signal (e.g. Ctrl+C). Re-raise it on
		// ourselves so callers see the real cause rather than a masked exit 0.
		process.kill(process.pid, signal);
		return;
	}
	process.exit(code ?? 1);
});
