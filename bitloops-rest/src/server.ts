import OpenTelemetry, { CounterName } from './opentelemetry/Opentelemetry';
OpenTelemetry.initialize();
import Services, { Options } from './services';
import { sleep } from './utils';
import { MQTopics, SSE_MESSAGE_TYPE } from './constants';
import { build } from './app';

let shutDownCalled = false;

const handleShutdown = async () => {
	// TODO see what error handling would be appropriate
	if (shutDownCalled) return;
	console.log('Server gracefully shuting down...');
	const services = Services.getServices();
	if (services) {
		const { mq, runningRequestsCache, imdb } = services;
		const topic = `${Options.getOption(MQTopics.SR_SSE_SERVER_TOPIC) ?? 'ssevent'}.*`;
		await mq.publish(topic, {
			type: { name: SSE_MESSAGE_TYPE.POD_SHUTDOWN, podId: Options.getServerUUID() },
		});
		console.info('Quitting mq connection...');
		await mq.gracefullyCloseConnection();
		console.info('Quitting imdb connection...');
		await imdb.closeConnection();

		console.info('Checking for active instances...');
		let keys = await runningRequestsCache.getCount();
		while (keys > 0) {
			await sleep(300);
			keys = await runningRequestsCache.getCount();
			console.info(`Server ${Options.getServerUUID()} still has active instances`);
		}
	}
	shutDownCalled = true;
};

//do something when app is closing
process.on('exit', async () => {
	console.info('exit');
	await handleShutdown();
});

//catches ctrl+c event
process.on('SIGINT', async () => {
	console.info('SIGINT');
	await handleShutdown();
	process.exit(1);
});

process.on('SIGTERM', async () => {
	console.info('SIGTERM');
	await handleShutdown();
	process.exit(1);
});

const start = async () => {
	try {
		const server = await build({ logger: false, trustProxy: true });
		console.log(`PORT: ${process.env.PORT || 8080}`);
		await server.listen(process.env.PORT || 8080, '0.0.0.0');
		const address = server.server.address();
		const family = typeof address === 'string' ? address : address?.family;
		const port = typeof address === 'string' ? address : address?.port;
		console.log(
			`${family} server ${typeof address === 'string' ? address : address?.address} started on port ${port}...`,
		);
	} catch (err) {
		// server.log.error(err);
		console.error(err);
		console.log('Exiting process...');
		process.exit(1);
	}
};
start();
