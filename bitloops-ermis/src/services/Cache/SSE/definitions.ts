import { ServerResponse } from 'http';

export type SSEConnectionReply = ServerResponse;

export type SSEUserConnectionInfo = {
    reply: SSEConnectionReply;
    id: string;
};

type workspaceId = string;

export type SSEWorkspaceConnections = Record<workspaceId, SSEUserConnectionInfo[]>;
