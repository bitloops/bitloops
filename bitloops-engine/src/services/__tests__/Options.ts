import Options from '../Options';
import { v4 as uuidv4 } from 'uuid';

describe('Options', () => {
	const SERVER_UUID: string = uuidv4();

	const RANDOM_OPTION: string = uuidv4();
	const RANDOM_OPTION_VALUE: string = uuidv4();
	const RANDOM_UNDEFINED_KEY: string = uuidv4();

	it('should set ServerUUID properly and read it afterwards', () => {
		Options.setServerUUID(SERVER_UUID);
		const result = Options.getServerUUID();
		expect(result).toBe(SERVER_UUID);
	});

	it('should set an option and read it without error', () => {
		Options.setOption(RANDOM_OPTION, RANDOM_OPTION_VALUE);
		const result = Options.getOption(RANDOM_OPTION);
		expect(result).toBe(RANDOM_OPTION_VALUE);
	});

	it('should return undefined when key is not set', () => {
		const result = Options.getOption(RANDOM_UNDEFINED_KEY);
		expect(result).toBeUndefined();
	});
});
