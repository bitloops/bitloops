// import {} from 'fastify';
import { ServerResponse } from 'http';

export type SSEConnectionReply = ServerResponse;

export type SSEUserConnectionInfo = {
	reply: SSEConnectionReply;
	id: string;
};

// type userId = string;
type workspaceId = string;
// export type SSEUserConnections = Record<userId, SSEUserConnectionInfo[]>;

export type SSEWorkspaceConnections = Record<workspaceId, SSEUserConnectionInfo[]>;
