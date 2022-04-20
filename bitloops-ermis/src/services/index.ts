import { v4 as uuid } from 'uuid';

import NATS from './MQ';
import Mongo from './Database';
import Options from './Options';
import {
    SSETopicToConnectionsCache,
    SSEConnectionToTopicsCache,
    SSEConnectionsCache,
    SubscriptionTopicsCache,
    XApiKeyCache,
} from './Cache';

import { IMQ } from './MQ/interfaces';
import { IDatabase } from './Database/interfaces';
import {
    ISSETopicToConnectionsCache,
    ISSEConnectionToTopicsCache,
    ISSEConnectionsCache,
    ISubscriptionTopicsCache,
    IXApiKeyCache,
} from './Cache/interfaces';

import { Services as TServices } from './definitions';
import { AppOptions } from '../constants';

class Services {
    private static sseTopicToConnectionsCache: ISSETopicToConnectionsCache = new SSETopicToConnectionsCache();
    private static sseConnectionToTopicsCache: ISSEConnectionToTopicsCache = new SSEConnectionToTopicsCache();
    private static sseConnectionsCache: ISSEConnectionsCache = new SSEConnectionsCache();
    private static subscriptionTopicsCache: ISubscriptionTopicsCache = new SubscriptionTopicsCache();
    private static xApiKeyCache: IXApiKeyCache = new XApiKeyCache(
        Options.getOptionAsNumber(AppOptions.MAX_X_API_KEY_CACHE, 1000),
    );

    private static mq: IMQ = new NATS();
    private static db: IDatabase = new Mongo();
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

        Options.setServerUUID(uuid());
        const services = {
            db: Services.db,
            mq: Services.mq,
            sseTopicToConnectionsCache: Services.sseTopicToConnectionsCache,
            sseConnectionToTopicsCache: Services.sseConnectionToTopicsCache,
            sseConnectionsCache: Services.sseConnectionsCache,
            subscriptionTopicsCache: Services.subscriptionTopicsCache,
            xApiKeyCache: Services.xApiKeyCache,
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
