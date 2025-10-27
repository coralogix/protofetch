#!/usr/bin/env node

import { readFileSync, writeFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const DEPRECATION_NOTICE = '> ⚠️ **DEPRECATION NOTICE**: This package has been replaced by `@coralogix/protofetch`. Please update your dependencies to use the new scoped package.\n\n';
const OLD_PACKAGE = 'cx-protofetch';

function getVersionFromCargo() {
	const cargoTomlPath = join(__dirname, '..', '..', 'Cargo.toml');
	const cargoToml = readFileSync(cargoTomlPath, 'utf-8');

	const versionMatch = cargoToml.match(/^version\s*=\s*"([^"]+)"/m);
	if (!versionMatch) {
		throw new Error('Could not find version in Cargo.toml');
	}

	return versionMatch[1];
}

function prepareReadme(packagePath) {
	const mainReadmePath = join(__dirname, '..', '..', 'README.md');
	const mainReadme = readFileSync(mainReadmePath, 'utf-8');

	const includeDeprecation = packagePath === OLD_PACKAGE;
	const content = includeDeprecation ? DEPRECATION_NOTICE + mainReadme : mainReadme;

	const readmePath = join(__dirname, '..', 'npm', packagePath, 'README.md');
	writeFileSync(readmePath, content, 'utf-8');

	console.log(`✓ README prepared for ${packagePath}${includeDeprecation ? ' (with deprecation notice)' : ''}`);
}

function updatePackageVersion(packagePath, version) {
	const packageJsonPath = join(__dirname, '..', 'npm', packagePath, 'package.json');
	const packageJson = readFileSync(packageJsonPath, 'utf-8');

	const updatedPackageJson = packageJson.replace('VERSION#TO#REPLACE', version);

	writeFileSync(packageJsonPath, updatedPackageJson, 'utf-8');

	console.log(`✓ Updated package.json version to ${version} for ${packagePath}`);
}
const { values } = parseArgs({
	options: {
		package: {
			type: 'string',
			short: 'p'
		}
	},
	strict: true
});

const packagePath = values.package;

if (!packagePath) {
	console.error('Error: Package path is required');
	console.error('Usage: node prepare-npm-package.js --package <package-path>');
	console.error('Example: node prepare-npm-package.js --package cx-protofetch');
	process.exit(1);
}

try {
	const version = getVersionFromCargo();
	console.log(`Preparing package ${packagePath} with version ${version}...`);

	prepareReadme(packagePath);
	updatePackageVersion(packagePath, version);

	console.log(`✓ Package ${packagePath} is ready for publishing`);
} catch (error) {
	console.error(`Error preparing package: ${error.message}`);
	process.exit(1);
}
