export type SSETopicsType = {
    topics: string[];
    creds: any;
};

export type SSEConnectionToTopicsCacheType = Record<string, SSETopicsType>;

export type SSEConnectionsType = Array<string>;

export type SSETopicToConnectionsCacheType = Record<string, SSEConnectionsType>;