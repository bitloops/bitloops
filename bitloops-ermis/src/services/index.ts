import { v4 as uuid } from 'uuid';

import NATS from './MQ';
import Options from './Options';
import {
    SSETopicToConnectionsCache,
    SSEConnectionToTopicsCache,
} from './Cache';

import { IMQ } from './MQ/interfaces';
import {
    ISSETopicToConnectionsCache,
    ISSEConnectionToTopicsCache,
} from './Cache/interfaces';

import { Services as TServices } from './definitions';

class Services {
    private static sseTopicToConnectionsCache: ISSETopicToConnectionsCache = new SSETopicToConnectionsCache();
    private static sseConnectionToTopicsCache: ISSEConnectionToTopicsCache = new SSEConnectionToTopicsCache();

    private static mq: IMQ = new NATS();
    private static services: TServices;

    static async initializeServices(): Promise<TServices> {
        console.info('Initializing MQ...');
        await Services.mq.initializeConnection();
        Options.setOption('mqReady', 'true');
        console.info('Connected to MQ!');

        Options.setServerUUID(uuid());
        const services = {
            mq: Services.mq,
            sseTopicToConnectionsCache: Services.sseTopicToConnectionsCache,
            sseConnectionToTopicsCache: Services.sseConnectionToTopicsCache,
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
