import { SSEUserConnectionInfo } from './definitions';

export interface ISSEConnectionsCache {
    cache(workspaceId: string, connection: SSEUserConnectionInfo);
    fetchWorkspaceConnections(workspaceId: string): SSEUserConnectionInfo[];
}