import NATS from '../NATS';
import { StringCodec, JSONCodec, NatsConnection, NatsError } from 'nats';

const CORRECT_SERVER = '127.0.0.1:4222';

// const NATS_ERROR_PAYLOAD: NatsError = {
// 	message: 'BAD_PAYLOAD',
// 	name: 'NatsError',
// 	code: 'BAD_PAYLOAD',
// };

describe('NATS', () => {
	jest.setTimeout(10000);
	let nats: NATS;

	describe('connecting', () => {
		it('should resolve nats connection', async () => {
			nats = new NATS({ servers: CORRECT_SERVER });
			return await expect(nats.getConnection()).resolves.not.toThrowError();
		});

		it('should resolve by returning existing connection', async () => {
			nats = new NATS({ servers: CORRECT_SERVER });
			await nats.getConnection();
			return await expect(nats.getConnection()).resolves.not.toThrowError();
		});

		it('should connect with default options', async () => {
			nats = new NATS();
			await expect(nats.getConnection()).resolves.not.toThrowError();
		});

		afterEach(async () => {
			await nats.closeConnection();
		});
	});

	describe('closing non-existing connection', () => {
		it('should reject with TypeError reading property close of undefined', async () => {
			nats = new NATS({ servers: CORRECT_SERVER });
			await expect(nats.closeConnection()).rejects.toThrowError(
				// TypeError("Cannot read property 'close' of undefined"),
			);
		});
	});

	// describe('failing to connect', () => {
	// 	it('should reject with error with invalid connection string', async () => {
	// 		expect.assertions(2);
	// 		nats = new NATS({
	// 			servers: '127.0.0.13:4223',
	// 			reconnect: false,
	// 			// maxReconnectAttempts: 0,
	// 			// verbose: true,
	// 			timeout: 500,
	// 			// reconnectTimeWait: 50,
	// 			// reconnectJitter: 50,
	// 			// reconnectJitterTLS: 50,
	// 			// reconnectDelayHandler: () => 50,
	// 			// pingInterval: 50,
	// 			// maxPingOut: 0,
	// 		});

	// 		try {
	// 			return await nats.getConnection();
	// 		} catch (error) {
	// 			expect(error.code).toBe('TIMEOUT');
	// 			expect(error.name).toBe('NatsError');
	// 		}
	// 	});
	// });

	describe('publishing', () => {
		/**
		 * @type {NatsConnection}
		 * Is needed to attach subscriber
		 */
		let nc: NatsConnection;

		const SUBJECT_NAME = 'random.subject';
		const STRING_PAYLOAD = 'hello world';
		const JSON_PAYLOAD = {
			data: 'hello world',
		};

		beforeAll(async () => {
			nats = new NATS({ servers: CORRECT_SERVER });
			nc = await nats.getConnection();

			const sc = StringCodec();
			const sub = nc.subscribe(SUBJECT_NAME);
			(async () => {
				for await (const m of sub) {
					// console.log(`[${sub.getProcessed()}]: ${sc.decode(m.data)}`);
					expect([STRING_PAYLOAD, JSON.stringify(JSON_PAYLOAD)]).toContain(sc.decode(m.data));
				}
				// console.log('subscription closed');
			})();
		});

		it('should publish a message and resolve', async () => {
			const sc = StringCodec();
			await expect(nats.publish(SUBJECT_NAME, STRING_PAYLOAD)).resolves.not.toThrowError();
		});

		it('should publish a JSON and resolve', async () => {
			const js = JSONCodec();
			await expect(nats.publish(SUBJECT_NAME, JSON_PAYLOAD)).resolves.not.toThrowError();
		});

		// it('should reject with error due to bad payload', async () => {
		// 	await expect(nats.publish(SUBJECT_NAME, 'not-encoded string')).rejects.not.toThrowError(NATS_ERROR_PAYLOAD);
		// 	await nats.publish(SUBJECT_NAME, 'not-encoded string').catch(console.error);
		// });

		afterAll(async () => {
			// await nats.closeConnection();
			// makes sure messages in flight are received first
			await nc.drain();
		});
	});
});
