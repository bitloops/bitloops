import Redis from 'ioredis';
import { Options } from '.';
import { CLOUD_PROVIDER, RedisSettings } from '../constants';
import { IIMDB, ITopicIdInfo, ITopicValueInfo } from './interfaces';

const username = Options.getOption(RedisSettings.REDIS_USERNAME);
const pass = Options.getOption(RedisSettings.REDIS_PASSWORD);
const host = Options.getOption(RedisSettings.REDIS_HOST);
const port = Options.getOption(RedisSettings.REDIS_PORT);
const cloudProvider = Options.getOption(CLOUD_PROVIDER);

let urlPrefix = 'redis';
if (cloudProvider) urlPrefix = 'rediss';

const envRedisOptions: Redis.RedisOptions = {
	host,
	port: port ? +port : undefined,
	username,
	password: pass,
};

const CONNECTION_ID_PREFIX = 'blsConnIds';
const POD_ID_PREFIX = 'blsPodIds';
const TOPIC_TO_CONNECTIONS_PREFIX = 'blsTopicToConnections';
const CONN_TO_TOPICS_PREFIX = 'blsConnIdsToTopics';

export default class RedisDB implements IIMDB {
	private client: Redis.Redis | Redis.Cluster;
	// private redisConnecting: Promise<any>;
	constructor(private redisOptions: any = envRedisOptions) {}

	async initializeConnection(): Promise<void> {
		let redisConnection: Redis.Redis | Redis.Cluster;
		if (cloudProvider === 'AWS') {
			console.log('CREATE CLUSTER ');
			redisConnection = new Redis.Cluster(
				[
					{
						host: Options.getOption(RedisSettings.NODE1_ENDPOINT),
						port: +port,
					},
					{
						host: Options.getOption(RedisSettings.NODE2_ENDPOINT),
						port: +port,
					},
				],
				{
					redisOptions: {
						tls: {
							checkServerIdentity: (/*host, cert*/) => {
								// skip certificate hostname validation
								return undefined;
							},
						},
						username,
						password: pass,
					},
				},
			);
			console.log('Cluster instance created!');
		} else {
			redisConnection = new Redis(this.redisOptions);
		}
		redisConnection.on('error', (err) => console.log('Redis Client Error', err));
		this.client = redisConnection;
		console.log('Redis is ready!');
	}

	async getConnection(): Promise<any> {
		if (this.client) return this.client;
		await this.initializeConnection();
		return this.client;
	}

	async closeConnection(): Promise<void> {
		console.info('Closing Redis connection');
		if (this.client) {
			await this.client.quit();
			this.client = null;
		} else {
			throw new Error('Connection already closed');
		}
	}

	async addConnectionIdToTopic(idInfo: ITopicIdInfo, connectionId: string): Promise<void> {
		const { workspaceId, topic } = idInfo;
		const hashId = `${TOPIC_TO_CONNECTIONS_PREFIX}:${workspaceId}:${topic}`;
		return this.saddAsync(hashId, connectionId);
	}

	async getConnectionIdsSubscribedToTopic(workspaceId: string, topic: string): Promise<string[]> {
		const key = `${TOPIC_TO_CONNECTIONS_PREFIX}:${workspaceId}:${topic}`;
		// return this.getHashValues(hashId);
		return this.smembers(key);
	}

	async addTopicsToConnectionId(connectionId: string, valueInfo: ITopicValueInfo): Promise<void> {
		const { workspaceId, topics } = valueInfo;
		const workspaceTopics = topics.map((topic) => `${workspaceId}:${topic}`);
		const key = `${CONN_TO_TOPICS_PREFIX}:${connectionId}`;
		return this.saddAsync(key, workspaceTopics);
	}

	async getConnectionIdValue(connectionId: string) {
		const hashId = `${CONNECTION_ID_PREFIX}:${connectionId}`;
		return this.hGetAll(hashId);
	}

	async storeConnectionIdValue(connectionId: string, keyValues: Record<string, string>): Promise<void> {
		const hashId = `${CONNECTION_ID_PREFIX}:${connectionId}`;
		await this.hMSet(hashId, keyValues);
	}

	/** TODO make transactional */
	async removeConnectionId(connectionId: string): Promise<void> {
		const key = `${CONN_TO_TOPICS_PREFIX}:${connectionId}`;
		const topics = await this.smembers(key);
		console.log('Topics for dead connection', topics);
		if (topics === null) return;
		await this.delAsync(key);
		/** Remove connectionId from topicToConnections */
		for (const topic of topics) {
			const key = `${TOPIC_TO_CONNECTIONS_PREFIX}:${topic}`;
			this.sremAsync(key, connectionId);
		}

		// remove connectionId from btlsPodIds
		const { podId } = await this.getConnectionIdValue(connectionId);
		const podIdKey = `${POD_ID_PREFIX}:${podId}`;
		await this.sremAsync(podIdKey, connectionId);

		const connectionKey = `${CONNECTION_ID_PREFIX}:${connectionId}`;
		await this.delAsync(connectionKey);
	}

	async addConnectionToPodId(podId: string, connectionId: string): Promise<void> {
		const key = `${POD_ID_PREFIX}:${podId}`;
		await this.client.sadd(key, connectionId);
	}

	/** TODO make transactional */
	async cleanPodState(podId: string): Promise<void> {
		const key = `${POD_ID_PREFIX}:${podId}`;
		const connectionIds = await this.client.smembers(key);
		console.log('Dead pod has following connections', connectionIds);
		// For all connections
		await Promise.all(
			connectionIds.map(async (connectionId) => {
				const connectionIdKey = `${CONNECTION_ID_PREFIX}:${connectionId}`;
				await this.client.del(connectionIdKey);
				const topicsKey = `${CONN_TO_TOPICS_PREFIX}:${connectionId}`;
				const topics = await this.client.smembers(topicsKey);
				// Remove connectionId to topics mapping
				await this.client.del(topicsKey);
				// For each topic remove the connection
				return Promise.all(
					topics.map((topic) => {
						const topicKey = `${TOPIC_TO_CONNECTIONS_PREFIX}:${topic}`;
						return this.client.srem(topicKey, connectionId);
					}),
				);
			}),
		);
		// remove podId
		const podKey = `${POD_ID_PREFIX}:${podId}`;
		await this.client.del(podKey);
	}

	async handleTopicUnsubscribe(connectionId: string, workspaceId: string, topic: string): Promise<void> {
		const workspaceTopic = `${workspaceId}:${topic}`;
		const key = `${CONN_TO_TOPICS_PREFIX}:${connectionId}`;
		/**
		 * 	returns with the number of removed members
		 *  returns 0 if key does not exist
		 */
		const n = await this.client.srem(key, workspaceTopic);

		const topicToConnectionKey = `${TOPIC_TO_CONNECTIONS_PREFIX}:${workspaceId}:${topic}`;
		const n2 = await this.client.srem(topicToConnectionKey, connectionId);
	}

	/** Adds a member or multiple members to a set */
	private async saddAsync(key: string, members: string | string[]) {
		await this.client.sadd(key, members);
	}

	private async sremAsync(key: string, members: string | string[]) {
		await this.client.srem(key, members);
	}

	/** Gets all the members in a set */
	private async smembers(key: string): Promise<string[]> {
		return this.client.smembers(key);
	}

	private async hGetAll(hashId: string): Promise<{ [key: string]: string }> {
		return this.client.hgetall(hashId);
	}

	// This command overwrites any specified fields already existing in the hash.
	private async hMSet(hashId: string, keyValuePair: Record<string, string>) {
		return this.client.hset(hashId, keyValuePair);
	}

	private async delAsync(key: string) {
		return this.client.del(key);
	}

	// private async hDel(hashId: string, key: string) {
	// 	const hDelAsync = promisify(this.client.hdel).bind(this.client);
	// 	return hDelAsync(hashId, key);
	// }

	// private async hLen(hashId: string): Promise<number> {
	// 	const hLenAsync = promisify(this.client.hlen).bind(this.client);
	// 	return hLenAsync(hashId);
	// }
}
