import { downloadBinary } from './getBinary.js';

if (process.argv.includes('install')) {
	// Check for --url argument for testing (only localhost allowed for security)
	const urlArg = process.argv.find(arg => arg.startsWith('--url='));
	let url = null;

	if (urlArg) {
		const providedUrl = urlArg.split('=')[1];
		try {
			const parsedUrl = new URL(providedUrl);
			// Only allow localhost URLs for testing
			if (parsedUrl.hostname === 'localhost' || parsedUrl.hostname === '127.0.0.1') {
				url = providedUrl;
			} else {
				console.error('Error: --url parameter only allows localhost URLs for security reasons');
				process.exit(1);
			}
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
