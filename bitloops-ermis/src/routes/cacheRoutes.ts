import { FastifyInstance } from 'fastify';
import { authMiddleware } from './helpers';
import { Services as TServices } from '../services/definitions';

import * as CacheController from '../controllers/cacheController';

const cacheRoutes = (services: TServices) => async (fastify: FastifyInstance, _options) => {
    fastify.decorate('services', services);
    fastify
        .get(
            '/:cacheType',
            {
                preHandler: authMiddleware,
            },
            CacheController.getCacheInfo,
        )
};

export default cacheRoutes;
