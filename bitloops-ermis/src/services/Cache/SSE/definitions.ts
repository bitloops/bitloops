import { AuthVerificationRequest, RawReply } from '../../../routes/definitions';

export type SSETopicsType = Array<string>;

export type SSEConnectionToTopicsCacheType = Record<string, SSETopicsType>;

export type SSEConnectionsType = Array<string>;

export type SSETopicToConnectionsCacheType = Record<string, SSEConnectionsType>;

export type SSEConnectionsCredsType = {
    creds: AuthVerificationRequest;
    connection: RawReply;
}

export type SSEConnectionsCacheType = Record<string, SSEConnectionsCredsType>;