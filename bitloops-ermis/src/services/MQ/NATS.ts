import {
    connect,
    ConnectionOptions,
    JSONCodec,
    NatsConnection,
    StringCodec,
    SubscriptionOptions,
    Msg,
    NatsError,
    Subscription,
} from 'nats';
import { AppOptions } from '../../constants';
import { IMQ } from './interfaces';
import Options from '../Options';

const getNatsOptions = (): ConnectionOptions => {
    let natsOptions: ConnectionOptions = {};
    const natsIp = Options.getOption(AppOptions.NATS_IP);
    const natsPort = Options.getOption(AppOptions.NATS_PORT);

    if (natsIp && natsPort) {
        natsOptions.servers = `${natsIp}:${natsPort}`;

        const natsUser = Options.getOption(AppOptions.NATS_USER);
        const natsPass = Options.getOption(AppOptions.NATS_PASSWORD);
        if (natsUser && natsPass) {
            natsOptions = {
                ...natsOptions,
                user: natsUser,
                pass: natsPass,
            };
        }
    }
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
     * Initializes the NATS connection
     * @returns NatsConnection
     */
    async initializeConnection(): Promise<NatsConnection> {
        this.natsConnection = await connect(this.options);
        return this.natsConnection;
    }

    /**
     * Returns the NATS connection
     * @returns NatsConnection
     */
    async getConnection(): Promise<NatsConnection> {
        if (this.natsConnection) return this.natsConnection;
        await this.initializeConnection();
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
        const message = await this.natsConnection.request(topic, JSONCodec().encode(body), { timeout: 10000 });
        const response = JSONCodec().decode(message.data) as T;
        // console.log('got response:', response);
        return response;
    }

    subscribe(topic: string, topicHandler: (data, subject: string) => void, subscriptionGroup: string): Subscription {
        const subscriptionParams: SubscriptionOptions = {
            callback: function (err: NatsError | null, msg: Msg) {
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
        return this.natsConnection.subscribe(topic, subscriptionParams);
    }
}

export default NATS;
