import { FastifyRequest, FastifyInstance, FastifyReply } from 'fastify';
import { Options } from '../services';

const healthyRoutes = async (fastify: FastifyInstance, _opts, done) => {
	fastify.get('/', async (request: FastifyRequest, reply: FastifyReply) => {
		if (Options.getOption('needsRestart') === 'true') reply.code(503).send('UNAVAILABLE');
		else reply.code(200).send(`OK ${Options.getServerUUID()}`);
	});
};

export default healthyRoutes;
