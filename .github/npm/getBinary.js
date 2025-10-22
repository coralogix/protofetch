import fetch from 'node-fetch';
import { mkdirSync, chmodSync, existsSync, readFileSync } from 'fs';
import { fileURLToPath } from 'url';
import { dirname, join } from 'path';
import { pipeline } from 'stream/promises';
import * as tar from 'tar';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

function getPlatform() {
	const type = process.platform;
	const arch = process.arch;

	if (type === 'win32' && arch === 'x64') {
		return 'x86_64-pc-windows-msvc';
	}

	if (type === 'linux' && arch === 'x64') {
		return 'x86_64-unknown-linux-musl';
	}

	if (type === 'linux' && arch === 'arm64') {
		return 'aarch64-unknown-linux-musl';
	}

	if (type === 'darwin' && arch === 'x64') {
		return 'x86_64-apple-darwin';
	}

	if (type === 'darwin' && arch === 'arm64') {
		return 'aarch64-apple-darwin';
	}

	throw new Error(`Unsupported platform: ${type} ${arch}. Please create an issue at https://github.com/coralogix/protofetch/issues`);
}

function getVersion() {
	const packageJsonPath = join(__dirname, 'package.json');
	const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf8'));
	return packageJson.version;
}

async function downloadBinary(options = {}) {
	const platform = getPlatform();
	const version = getVersion();

	// Support custom URL for testing (not exposed via postinstall, only via direct script call)
	const url = options.url || `https://github.com/coralogix/protofetch/releases/download/v${version}/protofetch_${platform}.tar.gz`;

	const binDir = join(__dirname, 'bin');
	const isWindows = process.platform === 'win32';
	const binaryName = isWindows ? 'protofetch.exe' : 'protofetch';
	const binaryPath = join(binDir, binaryName);

	if (!existsSync(binDir)) {
		mkdirSync(binDir, { recursive: true });
	}

	console.log(`Downloading protofetch binary from ${url}...`);

	let lastError;
	for (let attempt = 1; attempt <= 3; attempt++) {
		try {
			const response = await fetch(url, {
				redirect: 'follow',
				timeout: 60000
			});

			if (!response.ok) {
				throw new Error(`Failed to download binary (HTTP ${response.status}): ${response.statusText}`);
			}

			await pipeline(
				response.body,
				tar.extract({
					cwd: binDir,
					strip: 1,
					strict: true,
					preservePaths: false,
					preserveOwner: false,
					filter: (path, entry) => {
						const allowedFiles = ['protofetch', 'protofetch.exe'];
						const fileName = path.split('/').pop();
						return entry.type === 'File' && allowedFiles.includes(fileName);
					}
				})
			);

			if (!isWindows && existsSync(binaryPath)) {
				chmodSync(binaryPath, 0o755);
			}

			if (existsSync(binaryPath)) {
				console.log('protofetch binary installed successfully');
				return;
			} else {
				throw new Error(`Binary extraction failed - ${binaryName} not found after extraction`);
			}
		} catch (error) {
			lastError = error;
			if (attempt < 3) {
				console.log(`Download attempt ${attempt} failed, retrying...`);
				await new Promise(resolve => setTimeout(resolve, attempt * 1000));
			}
		}
	}

	throw new Error(`Failed to download protofetch after 3 attempts: ${lastError.message}`);
}

export { downloadBinary };
