import { IMQ } from '../services/interfaces/index';
import { FastifyInstance } from 'fastify';
import { authMiddleware } from './helpers';

import * as EventsController from '../controllers/events.controllers';

const injectedEventsRoutes = (mq: IMQ) => async (fastify: FastifyInstance, _options) => {
	fastify.decorate('mq', mq);
	fastify
		.post(
			'/subscribe/:connectionId',
			{
				// schema: {
				// 	// body: PostSubscribeEventsBody,
				// 	// params: PostSubscribeEventsParams,
				// 	// headers: AuthHeadersSchema,
				// },
				preHandler: authMiddleware,
			},
			EventsController.subscribeHandler,
		)
		.post(
			'/unsubscribe/:connectionId',
			{
				preHandler: authMiddleware,
			},
			EventsController.unsubscribeHandler,
		);
};

export default injectedEventsRoutes;
