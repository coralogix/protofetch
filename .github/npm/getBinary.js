import { Binary } from 'simple-binary-install';
import * as os from 'os';
import * as fs from 'fs';

function getPlatform() {
	const type = os.type();
	const arch = os.arch();

	if (type === 'Windows_NT' && arch === 'x64') {
		return 'x86_64-pc-windows-msvc';
	}

	if (type === 'Linux' && arch === 'x64') {
		return 'x86_64-unknown-linux-musl';
	}

	if (type === 'Linux' && arch === 'arm64') {
		return 'aarch64-unknown-linux-musl';
	}

	if (type === 'Darwin' && arch === 'x64') {
		return 'x86_64-apple-darwin';
	}

	if (type === 'Darwin' && arch === 'arm64') {
		return 'aarch64-apple-darwin';
	}

	throw new Error(`Unsupported platform: ${type} ${arch}. Please create an issue at https://github.com/coralogix/protofetch/issues`);
}

export function getBinary() {
	const platform = getPlatform();
	const { version } = JSON.parse(fs.readFileSync('./package.json'));
	const url = `https://github.com/coralogix/protofetch/releases/download/v${version}/protofetch_${platform}.tar.gz`;
	const name = 'protofetch';

	return new Binary(name, url)
}
