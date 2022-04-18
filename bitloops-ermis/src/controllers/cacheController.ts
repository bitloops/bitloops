import { FastifyReply, RouteHandlerMethod } from 'fastify';
import { CORS } from '../constants';
import { CacheRequest } from '../routes/definitions';
import { CacheTypeName } from '../services/Cache/definitions';
import { ISSEConnectionToTopicsCache, ISSETopicToConnectionsCache, ISSEConnectionsCache, ISubscriptionTopicsCache } from '../services/Cache/interfaces';

const HEADERS = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };
export const getCacheInfo: RouteHandlerMethod = async function (request: CacheRequest, reply: FastifyReply) {
    const { cacheType } = request.params;
    const { id } = request.query;

    let cache: ISSEConnectionToTopicsCache | ISSEConnectionsCache | ISSETopicToConnectionsCache | ISubscriptionTopicsCache;
    switch (cacheType) {
        case CacheTypeName.SSEConnectionToTopicsCache: {
            const { sseConnectionToTopicsCache } = this.services;
            cache = sseConnectionToTopicsCache as ISSEConnectionToTopicsCache;
            break;
        }
        case CacheTypeName.SSETopicToConnectionsCache: {
            const { sseTopicToConnectionsCache } = this.services;
            cache = sseTopicToConnectionsCache as ISSETopicToConnectionsCache;
            break;
        }
        case CacheTypeName.SSEConnectionsCache: {
            const { sseConnectionsCache } = this.services;
            cache = sseConnectionsCache as ISSEConnectionsCache;
            break;
        }
        case CacheTypeName.SubscriptionTopicsCache: {
            const { subscriptionTopicsCache } = this.services;
            cache = subscriptionTopicsCache as ISubscriptionTopicsCache;
            break;
        }
        default:
            console.error(`Cache type ${cacheType} - not implemented`);
            break;
    }

    let data;
    if (validCacheType(cacheType) && id) data = cache.fetch(id);
    else data = cache.fetchAll();
    console.log('data', data);
    reply.code(200).headers(HEADERS).send(data);
};

const validCacheType = (cacheType: string) => {
    return cacheType === CacheTypeName.SSEConnectionToTopicsCache || cacheType === CacheTypeName.SSETopicToConnectionsCache;
}