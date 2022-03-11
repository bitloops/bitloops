import Redis from 'ioredis';
import { Options } from '.';
import { CLOUD_PROVIDER, RedisSettings } from '../constants';
import { dataToBuffer, bufferToData } from '../utils/messagePack';
import { IIMDB } from './interfaces';

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

const AUTH_SESSION_PREFIX = 'blsAuthSession';

export default class RedisService implements IIMDB {
	private client: Redis.Redis | Redis.Cluster;
	constructor(private readonly redisOptions: Redis.RedisOptions = envRedisOptions) {}

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
			this.client.quit();
			this.client = null;
		} else {
			throw new Error('Connection already closed');
		}
	}

	async setSessionInfo(
		sessionState: string,
		sessionInfo: { sessionUuid: string; providerId: string; clientId: string; workspaceId: string },
	): Promise<void> {
		const key = `${AUTH_SESSION_PREFIX}:${sessionState}`;
		// EX stands for seconds
		await this.client.set(key, dataToBuffer(sessionInfo), 'EX', 1800);
	}

	async getSessionInfo(sessionState: string): Promise<{
		sessionUuid: string;
		providerId: string;
		clientId: string;
		workspaceId: string;
	}> {
		const key = `${AUTH_SESSION_PREFIX}:${sessionState}`;
		const buffer = await this.client.getBuffer(key);
		return bufferToData(buffer);
	}
}
