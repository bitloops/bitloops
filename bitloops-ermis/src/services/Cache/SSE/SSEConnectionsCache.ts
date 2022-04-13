import { ISSEConnectionsCache } from './interfaces';
import { SSEWorkspaceConnections, SSEUserConnectionInfo } from './definitions';

export default class SSEConnectionsCache implements ISSEConnectionsCache {
    private _cache: SSEWorkspaceConnections;

    constructor() {
        this._cache = {};
    }

    cache(workspaceId: string, connection: SSEUserConnectionInfo) {
        if (!this._cache[workspaceId]) this._cache[workspaceId] = [];
        this._cache[workspaceId].push(connection);
    }

    fetchWorkspaceConnections(workspaceId: string): SSEUserConnectionInfo[] {
        return this._cache[workspaceId];
    }
}
