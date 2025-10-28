#!/usr/bin/env node

import { readFileSync, writeFileSync, cpSync, rmSync, mkdirSync, readdirSync, statSync, copyFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const DEPRECATION_NOTICE = '> ⚠️ **DEPRECATION NOTICE**: This package has been replaced by `@coralogix/protofetch`. Please update your dependencies to use the new scoped package.\n\n';

const PACKAGES = {
	'cx-protofetch': {
		name: 'cx-protofetch',
		deprecated: true
	},
	'coralogix-protofetch': {
		name: '@coralogix/protofetch',
		deprecated: false
	}
};

const GENERATED_PACKAGES = Object.keys(PACKAGES);

const REPO_ROOT = join(__dirname, '..', '..');

function getVersionFromCargo() {
	const cargoToml = readFileSync(join(REPO_ROOT, 'Cargo.toml'), 'utf-8');
	const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
	if (!versionMatch) {
		throw new Error('Could not find version in Cargo.toml');
	}
	return versionMatch[1];
}

function preparePackage(packageKey, version) {
	const config = PACKAGES[packageKey];
	if (!config) {
		throw new Error(`Unknown package: ${packageKey}`);
	}

	const templatePath = join(__dirname, '..', 'npm');
	const outputPath = join(__dirname, '..', 'npm', packageKey);

	rmSync(outputPath, { recursive: true, force: true });
	mkdirSync(outputPath, { recursive: true });

	for (const item of readdirSync(templatePath)) {
		if (GENERATED_PACKAGES.includes(item)) {
			continue;
		}
		if (item === 'deprecation-notice.js' && !config.deprecated) {
			continue;
		}
		const srcPath = join(templatePath, item);
		const destPath = join(outputPath, item);

		if (statSync(srcPath).isDirectory()) {
			cpSync(srcPath, destPath, { recursive: true });
		} else {
			copyFileSync(srcPath, destPath);
		}
	}

	const packageJsonPath = join(outputPath, 'package.json');
	const packageJson = readFileSync(packageJsonPath, 'utf-8');
	const pkg = JSON.parse(packageJson);

	if (config.deprecated) {
		pkg.scripts.postinstall = 'node deprecation-notice.js && node scripts.js install';
	}

	const orderedPkg = {
		name: config.name,
		version: version,
		...pkg
	};

	writeFileSync(packageJsonPath, JSON.stringify(orderedPkg, null, 2) + '\n', 'utf-8');

	const mainReadme = readFileSync(join(REPO_ROOT, 'README.md'), 'utf-8');
	const readmeContent = config.deprecated ? DEPRECATION_NOTICE + mainReadme : mainReadme;
	writeFileSync(join(outputPath, 'README.md'), readmeContent, 'utf-8');

	console.log(`✓ Package ${packageKey} prepared (${config.name} v${version})${config.deprecated ? ' with deprecation notice' : ''}`);

	return outputPath;
}

const { values } = parseArgs({
	options: {
		package: {
			type: 'string',
			short: 'p'
		},
		version: {
			type: 'string',
			short: 'v'
		}
	},
	strict: true
});

const packageKey = values.package;

if (!packageKey || !PACKAGES[packageKey]) {
	console.error('Error: Valid package key is required');
	console.error('Usage: node prepare-npm-package.js --package <package-key> [--version <version>]');
	console.error(`Available packages: ${Object.keys(PACKAGES).join(', ')}`);
	process.exit(1);
}

try {
	const version = values.version || getVersionFromCargo();
	console.log(`Preparing package ${packageKey} with version ${version}...`);
	const outputPath = preparePackage(packageKey, version);
	console.log(`✓ Package ready at ${outputPath}`);
} catch (error) {
	console.error(`Error preparing package: ${error.message}`);
	process.exit(1);
}
