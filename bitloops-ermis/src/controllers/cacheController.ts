import { FastifyReply, RouteHandlerMethod } from 'fastify';
import { CacheRequest } from '../routes/definitions';
import { CacheTypeName } from '../services/Cache/definitions';
import { ISSEConnectionToTopicsCache, ISSETopicToConnectionsCache, ISSEConnectionsCache } from '../services/Cache/SSE/interfaces';

export const getCacheInfo: RouteHandlerMethod = async function (request: CacheRequest, reply: FastifyReply) {
    const { cacheType } = request.params;
    const { id } = request.query;

    let cacheTypeName: ISSEConnectionToTopicsCache | ISSEConnectionsCache | ISSETopicToConnectionsCache;
    switch (cacheType) {
        case CacheTypeName.SSEConnectionToTopicsCache: {
            const { sseConnectionToTopicsCache } = this.services;
            cacheTypeName = sseConnectionToTopicsCache as ISSEConnectionToTopicsCache;
            break;
        }
        default:
            console.error(`Cache type ${cacheType} - not implemented`);
            break;
    }

    cacheTypeName.fetch(id);
    reply.send();
};