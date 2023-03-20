const { Binary } = require('binary-install');
const os = require('os');

function getPlatform() {
	const type = os.type();
	const arch = os.arch();

	if (type === 'Windows_NT' && arch === 'x64') {
		return 'windows_amd64';
	}

	if (type === 'Linux' && arch === 'x64') {
		return 'x86_64-unknown-linux-musl';
	}

	if (type === 'Linux' && arch === 'arm64') {
		return 'aarch64-unknown-linux-musl';
	}

	if (type === 'Darwin' && arch === 'x64') {
		return 'darwin_amd64';
	}

	if (type === 'Darwin' && arch === 'arm64') {
		return 'darwin_arm64';
	}

	throw new Error(`Unsupported platform: ${type} ${arch}. Please create an issue at https://github.com/coralogix/protofetch/issues`);
}

function getBinary() {
	const platform = getPlatform();
	const version = require('./package.json').version;
	const url = `https://github.com/coralogix/protofetch/releases/download/v${ version }/protofetch_${ platform }.tar.gz`;
	const name = 'protofetch';

	return new Binary(name, url)
}

module.exports = getBinary;
