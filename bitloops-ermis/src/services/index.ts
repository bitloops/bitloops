import { v4 as uuid } from 'uuid';

import NATS from './MQ';
import Options from './Options';
import { SSEConnectionsCache } from './Cache';

import { IMQ } from './MQ/interfaces';
import {
    ISSEConnectionsCache,
} from './Cache/interfaces';

import { Services as TServices } from './definitions';

class Services {
    private static sseConnectionsCache: ISSEConnectionsCache = new SSEConnectionsCache();
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
            sseConnectionsCache: Services.sseConnectionsCache,
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
