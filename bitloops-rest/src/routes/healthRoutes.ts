import { FastifyRequest, FastifyInstance, FastifyReply } from 'fastify';
import { CounterName } from '../opentelemetry/Opentelemetry';
import { Options } from '../services';

const healthyRoutes = async (fastify: FastifyInstance, _opts, done) => {
	fastify.get('/', async function (request: FastifyRequest, reply: FastifyReply) {
		this.openTelemetry.increaseCounter(CounterName.RUNNING_INSTANCES);
		if (Options.getOption('needsRestart') === 'true') reply.code(503).send('UNAVAILABLE');
		else reply.code(200).send(`OK ${Options.getServerUUID()}`);
	});
};

export default healthyRoutes;
