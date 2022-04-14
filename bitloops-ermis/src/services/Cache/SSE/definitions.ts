export type SSETopicsType = {
    topics: string[];
    creds: any; //TODO maybe delete creds from here
};

export type SSEConnectionToTopicsCacheType = Record<string, SSETopicsType>;

export type SSEConnectionsType = Array<string>;

export type SSETopicToConnectionsCacheType = Record<string, SSEConnectionsType>;

export type SSEConnectionsCredsType = {
    creds: any;
    connection: any;
}

export type SSEConnectionsCacheType = Record<string, SSEConnectionsCredsType>;