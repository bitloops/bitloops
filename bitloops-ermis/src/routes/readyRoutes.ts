import { FastifyInstance, FastifyReply } from 'fastify';
import { Options } from '../services';

const readyRoutes = async (fastify: FastifyInstance, _opts) => {
	fastify.get('/', async (_, reply: FastifyReply) => {
		if (Options.getOption('mqReady') === 'true' && Options.getOption('dbReady') === 'true')
			reply.code(200).send('OK');
		else {
			if (Options.getOption('mqReady') !== 'true') console.info('MQ is not ready...');
			if (Options.getOption('dbReady') !== 'true') console.info('DB is not ready...');
			reply.code(503).send('UNAVAILABLE');
		}
	});
};

export default readyRoutes;
