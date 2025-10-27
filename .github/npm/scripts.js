import { parseArgs } from 'node:util';
import { downloadBinary } from './getBinary.js';

function isLocalhost(hostname) {
	return hostname === 'localhost' || hostname === '127.0.0.1';
}

if (process.argv.includes('install')) {
	const { values } = parseArgs({
		options: {
			url: {
				type: 'string'
			}
		},
		strict: false
	});

	let url = null;

	if (values.url) {
		try {
			const parsedUrl = new URL(values.url);
			if (!isLocalhost(parsedUrl.hostname)) {
				console.error('Error: --url parameter only allows localhost URLs for security reasons');
				process.exit(1);
			}
			url = values.url;
		} catch (error) {
			console.error('Error: Invalid URL provided to --url parameter');
			process.exit(1);
		}
	}

	downloadBinary({ url })
		.then(() => {
			console.log('Installation complete');
			process.exit(0);
		})
		.catch((error) => {
			console.error('Installation failed:', error.message);
			process.exit(1);
		});
}
