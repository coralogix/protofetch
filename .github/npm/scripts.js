import { getBinary } from './getBinary.js';

if (process.argv.includes('install')) {
	getBinary().install();
}
