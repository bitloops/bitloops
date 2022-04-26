/**
 * All plugins and routes are
 * initialized here
 */
import fastify, { FastifyReply, FastifyServerOptions } from 'fastify';
import formBodyPlugin from 'fastify-formbody';
import fastifyStaticPlugin from 'fastify-static';
import * as path from 'path';
import Services from './services';
import {
    healthRoutes,
    readyRoutes,
    eventsRoutes,
    cacheRoutes,
} from './routes';
import { CORS } from './constants';
import SubscriptionEvents from './handlers/SubscriptionEvents';

import { requestEventRequest } from './routes/definitions';

const optionsHandler = async (request: requestEventRequest, reply: FastifyReply) => {
    reply
        .code(200)
        .headers({
            [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN,
            [CORS.HEADERS.ACCESS_CONTROL_ALLOW_HEADERS]: CORS.ALLOW_HEADERS,
        })
        .send('OK');
};

const build = async (opts: FastifyServerOptions = {}) => {
    const server = fastify(opts);
    const services = await Services.initializeServices();
    const { mq, subscriptionTopicsCache } = services;
    const subscriptionEvents = new SubscriptionEvents(mq, subscriptionTopicsCache);

    server
        .register(formBodyPlugin)
        .register(fastifyStaticPlugin, {
            root: path.join(__dirname, 'public'),
        })
        .options('*', optionsHandler)
        .register(healthRoutes, { prefix: '/healthy' })
        .register(readyRoutes, { prefix: '/ready' })
        .register(eventsRoutes(services, subscriptionEvents), { prefix: '/bitloops/events' })
        .register(cacheRoutes(services), { prefix: '/bitloops/cache' })
        .setNotFoundHandler((_req, reply) => {
            reply.status(404).send('Route not found');
        });

    return server;
};

export { build };
