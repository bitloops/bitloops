import { IPublicKeysCache } from './interfaces/index';
import { v4 as uuid } from 'uuid';

import NATS from './NATS';
import {
	RunningRequestsCache,
	SecretCache,
	XApiKeyCache,
	WorkflowEventTriggerCache,
	FirebaseConnectionsCache,
	FirebaseTokensCache,
	PublicKeysCache,
} from './Cache';
import Options from './Options';
import { Mongo } from './mongo';
import Logger from './Logger';
import {
	IMQ,
	IDatabase,
	ILogger,
	IRunningRequestsCache,
	ISecretCache,
	IXApiKeyCache,
	IWorkflowEventTriggerCache,
	IFirebaseConnectionsCache,
	IFirebaseTokensCache,
	IIMDB,
} from './interfaces';

import { Services as TServices } from '../services/definitions';
import { AppOptions } from '../constants';
import Redis from './Redis';

enum ServicesOptions {
	MAX_SECRETS_CACHE = 'MAX_SECRETS_CACHE',
	MAX_EVENT_TRIGGERS_CACHE = 'MAX_EVENT_TRIGGERS_CACHE',
	MAX_FIREBASE_CONNECTIONS_CACHE = 'MAX_FIREBASE_CONNECTIONS_CACHE',
	MAX_FIREBASE_TOKENS_CACHE = 'MAX_FIREBASE_TOKENS_CACHE',
	MAX_PUBLIC_KEYS_CACHE = 'MAX_PUBLIC_KEYS_CACHE',
}

class Services {
	private static runningRequestsCache: IRunningRequestsCache = new RunningRequestsCache();
	private static secretCache: ISecretCache = new SecretCache(
		Options.getOptionAsNumber(ServicesOptions.MAX_SECRETS_CACHE, 1000),
	);
	private static publicKeysCache: IPublicKeysCache = new PublicKeysCache(
		Options.getOptionAsNumber(ServicesOptions.MAX_PUBLIC_KEYS_CACHE, 1000),
	);
	private static xApiKeyCache: IXApiKeyCache = new XApiKeyCache(
		Options.getOptionAsNumber(AppOptions.MAX_X_API_KEY_CACHE, 1000),
	);
	private static workflowEventTriggerCache: IWorkflowEventTriggerCache = new WorkflowEventTriggerCache(
		Options.getOptionAsNumber(ServicesOptions.MAX_EVENT_TRIGGERS_CACHE, 1000),
	);
	private static firebaseConnectionsCache: IFirebaseConnectionsCache = new FirebaseConnectionsCache(
		Options.getOptionAsNumber(ServicesOptions.MAX_FIREBASE_CONNECTIONS_CACHE, 1000),
	);
	private static firebaseTokensCache: IFirebaseTokensCache = new FirebaseTokensCache(
		Options.getOptionAsNumber(ServicesOptions.MAX_FIREBASE_CONNECTIONS_CACHE, 1000),
	);
	private static mq: IMQ = new NATS(Options.getOption('NATS_IP') ? undefined : {});
	private static db: IDatabase = new Mongo();
	private static imdb: IIMDB = new Redis();
	private static logger: ILogger = new Logger(Services.mq);
	private static services: TServices;

	static async initializeServices(): Promise<TServices> {
		console.info('Initializing DB...');
		await Services.db.connect();
		Options.setOption('dbReady', 'true');
		console.info('Connected to DB!');
		console.info('Initializing MQ...');
		await Services.mq.initializeConnection();
		Options.setOption('mqReady', 'true');
		console.info('Connected to MQ!');

		console.info('Initializing IMDB...');
		await Services.imdb.initializeConnection();
		Options.setOption('imdbReady', 'true');
		console.info('Connected to IMDB!');
		Options.setServerUUID(uuid());
		const services = {
			db: Services.db,
			logger: Services.logger,
			mq: Services.mq,
			imdb: Services.imdb,
			firebaseConnectionsCache: Services.firebaseConnectionsCache,
			firebaseTokensCache: Services.firebaseTokensCache,
			runningRequestsCache: Services.runningRequestsCache,
			secretCache: Services.secretCache,
			xApiKeyCache: Services.xApiKeyCache,
			workflowEventTriggerCache: Services.workflowEventTriggerCache,
			publicKeysCache: Services.publicKeysCache,
			Options: Options,
		};
		Services.services = services;
		return services;
	}

	static getServices(): TServices {
		return Services.services;
	}
}

export default Services;
export { Options };
