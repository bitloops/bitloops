import {
	connect,
	ConnectionOptions,
	JSONCodec,
	NatsConnection,
	StringCodec,
	SubscriptionOptions,
	Msg,
	NatsError,
} from 'nats';
import { MQSubscriptionCallbackFunc } from './definitions';
import { IMQ } from './interfaces';
import Options from './Options';

const getNatsOptions = (): ConnectionOptions => {
	let natsOptions;
	if (process.env.NATS_IP && process.env.NATS_PORT) {
		natsOptions = {
			servers: `${Options.getOption('NATS_IP')}:${Options.getOption('NATS_PORT')}`,
		};
		if (process.env.NATS_USER && process.env.NATS_PASSWORD) {
			natsOptions = {
				servers: `${Options.getOption('NATS_IP')}:${Options.getOption('NATS_PORT')}`,
				user: Options.getOption('NATS_USER'),
				pass: Options.getOption('NATS_PASSWORD'),
			};
		}
	} else natsOptions = {};
	return natsOptions;
};

const natsOptions = getNatsOptions();

/**
 * Handles the connection, publishing and gracefull shutdown
 * of NATS
 */
class NATS implements IMQ {
	static JSONCodec = JSONCodec;
	static StringCodec = StringCodec;
	private options: ConnectionOptions;
	private natsConnection: NatsConnection;

	constructor(options: ConnectionOptions = natsOptions) {
		this.options = options;
	}

	/**
	 * Returns the NATS connection
	 * @returns NatsConnection
	 */
	async getConnection(): Promise<NatsConnection> {
		if (this.natsConnection) return this.natsConnection;
		this.natsConnection = await connect(this.options);
		return this.natsConnection;
	}

	/**
	 * Closes the NATS connection
	 * @returns void
	 */
	async closeConnection(): Promise<void> {
		return await this.natsConnection.close();
	}

	/**
	 * Gracefully closes the connection after draining it
	 */
	async gracefullyCloseConnection(): Promise<void> {
		if (this.natsConnection) {
			await this.natsConnection.drain();
			this.natsConnection = null;
		}
	}

	/**
	 * Publishes a treated message to the specified topic
	 * @param topic
	 * @param message
	 */
	async publish(topic: string, message: Record<string, unknown> | string): Promise<void> {
		if (!this.natsConnection) throw new Error('Nats connection not established');

		let encodedMsg;
		if (typeof message === 'string') {
			encodedMsg = StringCodec().encode(message);
		} else if (typeof message === 'object') {
			encodedMsg = JSONCodec().encode(message);
		}
		this.natsConnection.publish(topic, encodedMsg);
	}

	async request<T>(topic: string, body: any): Promise<T> {
		const message = await this.natsConnection.request(topic, JSONCodec().encode(body), { timeout: 60000 });
		const response = JSONCodec().decode(message.data) as T;
		// console.log('got response:', response);
		return response;
	}

	async subscribe(topic: string, topicHandler?: MQSubscriptionCallbackFunc, subscriptionGroup?: string) {
		const subscriptionParams: SubscriptionOptions = {
			callback: (err: NatsError | null, msg: Msg) => {
				if (err) {
					console.error('NatsError', err);
					return;
				}
				const jc = JSONCodec();
				const message = jc.decode(msg.data);
				if (msg.reply) {
					message['originalReply'] = msg.reply;
				}
				topicHandler(message, msg.subject);
			},
		};
		if (subscriptionGroup) {
			subscriptionParams.queue = subscriptionGroup;
		}
		this.natsConnection.subscribe(topic, subscriptionParams);
	}
}

export default NATS;
