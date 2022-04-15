import { AuthVerificationRequest, RawReply } from '../../../routes/definitions';
import { SSETopicsType, SSEConnectionsType, SSEConnectionsCredsType } from './definitions';

export interface ICache<T> {
    fetch(id: string): T;
}

export interface ISSEConnectionToTopicsCache extends ICache<SSETopicsType> {
    cache(connectionId: string, topic: string);
    // fetch(connectionId: string): SSETopicsType;
    delete(connectionId: string);
    deleteTopicFromConnectionId(connectionId: string, topic: string)
}

export interface ISSETopicToConnectionsCache extends ICache<SSEConnectionsType> {
    cache(topic: string, connectionId: string);
    // fetch(topic: string): SSEConnectionsType;
    delete(topic: string);
    deleteConnectionIdFromTopic(topic: string, connectionId: string)
}

export interface ISSEConnectionsCache extends ICache<SSEConnectionsCredsType> {
    cache(connectionId: string, connection: RawReply, creds: AuthVerificationRequest);
    // fetch(connectionId: string): SSEConnectionsCredsType;
    delete(connectionId: string)
}