#!/usr/bin/env node

import { readFileSync, writeFileSync, rmSync, mkdirSync, cpSync, chmodSync, existsSync } from 'node:fs';
import { join, dirname, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';
import { execFileSync } from 'node:child_process';

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = join(__dirname, '..', '..');
const SRC_DIR = join(__dirname, 'src');
const DIST_DIR = join(__dirname, 'dist');

const DEPRECATION_NOTICE = '> ⚠️ **DEPRECATION NOTICE**: This package has been replaced by `@coralogix/protofetch`. Please update your dependencies to use the new scoped package.\n\n';

// Single source of truth for the rust target -> npm platform package mapping.
// Mirrors the PLATFORM_PACKAGES table in src/run.js (keep them in sync).
//
// The two linux targets are static musl binaries that run on both glibc and
// musl hosts, so there is one linux package per arch and NO `libc` field — a
// `libc` filter would make npm skip the package on glibc systems, which is
// wrong for a static binary. If a glibc-dynamic build is ever added, switch to
// the napi-rs convention (`linux-x64-gnu` / `linux-x64-musl`) with `libc` set.
const PLATFORMS = [
	{ rust: 'aarch64-apple-darwin', node: 'darwin-arm64', os: 'darwin', cpu: 'arm64', binary: 'protofetch' },
	{ rust: 'x86_64-apple-darwin', node: 'darwin-x64', os: 'darwin', cpu: 'x64', binary: 'protofetch' },
	{ rust: 'aarch64-unknown-linux-musl', node: 'linux-arm64', os: 'linux', cpu: 'arm64', binary: 'protofetch' },
	{ rust: 'x86_64-unknown-linux-musl', node: 'linux-x64', os: 'linux', cpu: 'x64', binary: 'protofetch' },
	{ rust: 'x86_64-pc-windows-msvc', node: 'win32-x64', os: 'win32', cpu: 'x64', binary: 'protofetch.exe' }
];

const platformPackageName = node => `@coralogix/protofetch-${node}`;

function getVersionFromCargo() {
	const cargoToml = readFileSync(join(REPO_ROOT, 'Cargo.toml'), 'utf-8');
	const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
	if (!versionMatch) {
		throw new Error('Could not find version in Cargo.toml');
	}
	return versionMatch[1];
}

// The optionalDependencies map shared by the main and cx mirror packages. Every
// platform package is pinned to the EXACT version so a given main version only
// ever resolves the matching platform binaries.
function optionalDependencies(version) {
	const deps = {};
	for (const p of PLATFORMS) {
		deps[platformPackageName(p.node)] = version;
	}
	return deps;
}

function freshDir(path) {
	rmSync(path, { recursive: true, force: true });
	mkdirSync(path, { recursive: true });
}

function readme(deprecated) {
	const mainReadme = readFileSync(join(REPO_ROOT, 'README.md'), 'utf-8');
	return deprecated ? DEPRECATION_NOTICE + mainReadme : mainReadme;
}

// Ship the repository LICENSE in every published package. npm always includes a
// file named LICENSE regardless of the `files` allow-list, so platform packages
// pick it up too.
function copyLicense(outputPath) {
	cpSync(join(REPO_ROOT, 'LICENSE'), join(outputPath, 'LICENSE'));
}

// Generates the user-facing package (`@coralogix/protofetch` or the deprecated
// `cx-protofetch` mirror): a `bin` wrapper plus the optionalDependencies map.
// Both share the same platform packages; the only differences are the name and
// the deprecation banner/notice.
function prepareEntryPackage({ name, version, deprecated, outputPath }) {
	freshDir(outputPath);

	cpSync(join(SRC_DIR, 'run.js'), join(outputPath, 'run.js'));

	const template = JSON.parse(readFileSync(join(SRC_DIR, 'package.json'), 'utf-8'));

	const pkg = {
		name,
		version,
		...template,
		optionalDependencies: optionalDependencies(version)
	};

	if (deprecated) {
		cpSync(join(SRC_DIR, 'deprecation-notice.js'), join(outputPath, 'deprecation-notice.js'));
		pkg.scripts = { ...pkg.scripts, postinstall: 'node deprecation-notice.js' };
	}

	writeFileSync(join(outputPath, 'package.json'), JSON.stringify(pkg, null, 2) + '\n', 'utf-8');
	writeFileSync(join(outputPath, 'README.md'), readme(deprecated), 'utf-8');
	copyLicense(outputPath);

	console.log(`✓ Package ${name} prepared (v${version})${deprecated ? ' with deprecation notice' : ''}`);
	return outputPath;
}

function prepareMainPackage(version) {
	return prepareEntryPackage({
		name: '@coralogix/protofetch',
		version,
		deprecated: false,
		outputPath: join(DIST_DIR, 'coralogix-protofetch')
	});
}

function prepareCxMirror(version) {
	return prepareEntryPackage({
		name: 'cx-protofetch',
		version,
		deprecated: true,
		outputPath: join(DIST_DIR, 'cx-protofetch')
	});
}

// Extracts the prebuilt binary for `platform` from the artifacts directory into
// the platform package's bin/. Artifacts are the CI tarballs
// (protofetch_<rust-target>.tar.gz, each containing bin/protofetch[.exe]). We
// shell out to the system `tar` so this script needs no npm dependencies.
function extractBinary(platform, artifactsDir, outputPath) {
	const tarball = join(artifactsDir, `protofetch_${platform.rust}.tar.gz`);
	if (!existsSync(tarball)) {
		throw new Error(`Missing artifact for ${platform.rust}: expected ${tarball}`);
	}

	// Hand tar RELATIVE, forward-slash paths (relative to the process cwd). A
	// Windows absolute path has a drive-letter colon (D:\...), which GNU tar
	// treats as a remote "host:path" and fails to open; a relative forward-slash
	// path avoids that and works with both GNU tar (Windows) and bsdtar
	// (macOS/Linux). The tarball contains a top-level bin/ directory, so
	// extracting into outputPath yields outputPath/bin/protofetch[.exe].
	const cwd = process.cwd();
	const relTarball = relative(cwd, resolve(tarball)).split(sep).join('/');
	const relOutput = relative(cwd, outputPath).split(sep).join('/');
	execFileSync('tar', ['-xzf', relTarball, '-C', relOutput], { stdio: 'inherit' });

	const binaryPath = join(outputPath, 'bin', platform.binary);
	if (!existsSync(binaryPath)) {
		throw new Error(`Binary ${platform.binary} not found after extracting ${tarball}`);
	}
	if (platform.os !== 'win32') {
		chmodSync(binaryPath, 0o755);
	}
}

function preparePlatformPackage(platform, version, artifactsDir) {
	const name = platformPackageName(platform.node);
	const outputPath = join(DIST_DIR, 'platform', `coralogix-protofetch-${platform.node}`);

	freshDir(outputPath);

	// Platform packages are pure binary carriers: NO `bin` field. The command is
	// provided solely by the main package's run.js wrapper, which resolves the
	// binary here at runtime. Declaring `bin` on both the main package and every
	// platform package would create a `protofetch` bin-name collision (npm would
	// pick a winner nondeterministically and could bypass the wrapper, losing the
	// PROTOFETCH_BINARY_PATH override and the friendly missing-binary error).
	// npm preserves the executable bit on the packed file regardless of `bin`.
	const binRelative = `bin/${platform.binary}`;
	const pkg = {
		name,
		version,
		description: `Prebuilt protofetch binary for ${platform.node}.`,
		repository: {
			type: 'git',
			url: 'git+https://github.com/coralogix/protofetch.git'
		},
		homepage: 'https://github.com/coralogix/protofetch',
		license: 'Apache-2.0',
		os: [platform.os],
		cpu: [platform.cpu],
		publishConfig: { access: 'public' },
		files: [binRelative]
	};

	writeFileSync(join(outputPath, 'package.json'), JSON.stringify(pkg, null, 2) + '\n', 'utf-8');
	writeFileSync(join(outputPath, 'README.md'), readme(false), 'utf-8');
	copyLicense(outputPath);

	extractBinary(platform, artifactsDir, outputPath);

	console.log(`✓ Package ${name} prepared (v${version})`);
	return outputPath;
}

function resolvePlatform(rust) {
	const platform = PLATFORMS.find(p => p.rust === rust);
	if (!platform) {
		throw new Error(`Unknown rust target: ${rust}. Known: ${PLATFORMS.map(p => p.rust).join(', ')}`);
	}
	return platform;
}

const { values } = parseArgs({
	options: {
		target: { type: 'string', short: 't' },
		rust: { type: 'string', short: 'r' },
		artifacts: { type: 'string', short: 'a' },
		version: { type: 'string', short: 'v' }
	},
	strict: true
});

function usage() {
	console.error('Usage: node prepare-package.js --target <main|cx|platform|all> [options]');
	console.error('');
	console.error('  --target, -t     What to generate: main, cx, platform, or all');
	console.error('  --rust, -r       Rust target triple (required for --target platform)');
	console.error('  --artifacts, -a  Directory containing protofetch_<target>.tar.gz files');
	console.error('                   (required for --target platform and --target all)');
	console.error('  --version, -v    Version to stamp (defaults to the Cargo.toml version)');
	console.error('');
	console.error(`Known rust targets: ${PLATFORMS.map(p => p.rust).join(', ')}`);
}

try {
	const target = values.target;
	if (!target || !['main', 'cx', 'platform', 'all'].includes(target)) {
		usage();
		process.exit(1);
	}

	const version = values.version || getVersionFromCargo();
	console.log(`Preparing "${target}" with version ${version}...`);

	if (target === 'main') {
		prepareMainPackage(version);
	} else if (target === 'cx') {
		prepareCxMirror(version);
	} else if (target === 'platform') {
		if (!values.rust) {
			throw new Error('--rust <target> is required for --target platform');
		}
		if (!values.artifacts) {
			throw new Error('--artifacts <dir> is required for --target platform');
		}
		preparePlatformPackage(resolvePlatform(values.rust), version, values.artifacts);
	} else if (target === 'all') {
		if (!values.artifacts) {
			throw new Error('--artifacts <dir> is required for --target all');
		}
		prepareMainPackage(version);
		prepareCxMirror(version);
		for (const platform of PLATFORMS) {
			preparePlatformPackage(platform, version, values.artifacts);
		}
	}

	console.log('✓ Done');
} catch (error) {
	console.error(`Error preparing package: ${error.message}`);
	process.exit(1);
}
