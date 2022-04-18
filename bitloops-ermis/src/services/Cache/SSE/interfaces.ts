import { AuthVerificationRequest, RawReply } from '../../../routes/definitions';
import { ICache } from '../interfaces';
import { SSETopicsType, SSEConnectionsType, SSEConnectionsCredsType, SSEConnectionsCacheType } from './definitions';

export interface ISSEConnectionToTopicsCache extends ICache<SSETopicsType> {
    cache(connectionId: string, topic: string);
    delete(connectionId: string);
    deleteTopicFromConnectionId(connectionId: string, topic: string)
}

export interface ISSETopicToConnectionsCache extends ICache<SSEConnectionsType> {
    cache(topic: string, connectionId: string);
    delete(topic: string);
    deleteConnectionIdFromTopic(topic: string, connectionId: string)
}

export interface ISSEConnectionsCache extends ICache<SSEConnectionsCredsType> {
    cache(connectionId: string, connection: RawReply, creds: AuthVerificationRequest);
    delete(connectionId: string);
}