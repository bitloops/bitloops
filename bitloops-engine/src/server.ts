// import { WebSocketServer } from 'ws';
import init from './tracing';
const tracing = init('bitloops-engine', 'development');
import { healthyHandler, readyHandler, cachesHandler } from './httpHandlers';
import express from 'express';
import { Admin, BitloopsEngine, GRPC, REST, Message } from './handlers';
import Services, { Options } from './services';
import { MQTopics, ServerSettings } from './constants';
import SubscriptionRouter from './handlers/sr/SubscriptionRouter';

let shutDownCalled = false;

const handleShutdown = async () => {
	// TODO see what error handling would be appropriate
	if (shutDownCalled) return;
	const services = Services.getServices();
	if (services) {
		const { mq, runningWorkflowInstanceCache, db, imdb } = services;
		console.info('Quitting mq connection...');
		await mq.gracefullyCloseConnection();
		console.info('Quitting db connection...');
		await db.disconnect();
		console.info('Quitting imdb connection...');
		await imdb.closeConnection();

		console.info('Checking for active instances...');
		let keys = await runningWorkflowInstanceCache.getCount();
		while (keys > 0) {
			await sleep(300);
			keys = await runningWorkflowInstanceCache.getCount();
			console.info(`Server ${Options.getServerUUID()} still has active instances`);
		}
	}
	shutDownCalled = true;
};

const sleep = (ms: number) => {
	return new Promise((resolve) => setTimeout(resolve, ms));
};

const app = express();

/**
 * Healthy responds to Kubernetes about the health status of the application.
 * If the service requires a restart then it responds with a 503, if its fine 200.
 */
app.get('/healthy', healthyHandler);
app.get('/ready', readyHandler);

app.listen(Options.getOption(ServerSettings.SERVICE_PORT) || ServerSettings.DEFAULT_SERVER_PORT, () => {
	console.info(
		`The application is listening on port ${
			Options.getOption('SERVICE_PORT') || ServerSettings.DEFAULT_SERVER_PORT
		}!`,
	);
});

(async () => {
	const services = await Services.initializeServices();
	app.get('/caches', cachesHandler(services));
	const { mq, imdb } = services;
	// const bitloopsEngine = new BitloopsEngine(services);
	const bitloopsEngine = new BitloopsEngine(services);
	// const events = new Events(services);
	const grpc = new GRPC(mq);
	const rest = new REST(mq);
	const admin = new Admin(services);
	const sr = new SubscriptionRouter(imdb, mq);
	const message = new Message(mq);

	// A service is a subscriber that listens for messages, and responds
	const version = Options.getVersion();
	const engineQueue = Options.getOption(MQTopics.BITLOOPS_ENGINE_QUEUE);
	const engineEventsTopic = Options.getOption(MQTopics.ENGINE_EVENTS_TOPIC);
	mq.subscribe(engineEventsTopic, (msg) => bitloopsEngine.handleEventsTopic(msg), engineQueue);

	// TODO Add Error Handling for missing workflow and/or node
	const engineTopic = `${version}.${Options.getOption(MQTopics.ENGINE_TOPIC)}`;
	console.log('server engineTopic', engineTopic);
	mq.subscribe(engineTopic, (msg) => bitloopsEngine.handleEngineTopic(msg), engineQueue);

	const engineGRPCTopic = `${version}.${Options.getOption(MQTopics.ENGINE_GRPC_TOPIC)}`;
	mq.subscribe(engineGRPCTopic, (msg) => grpc.callback(msg), engineQueue);

	const engineRESTTopic = `${version}.${Options.getOption(MQTopics.ENGINE_REST_TOPIC)}`;
	mq.subscribe(engineRESTTopic, (msg) => rest.callback(msg), engineQueue);

	const engineMessageTopic = `${version}.${Options.getOption(MQTopics.ENGINE_MESSAGE_TOPIC)}`;
	mq.subscribe(engineMessageTopic, (msg) => message.callback(msg), engineQueue);

	const engineAdminTopicVersion = `${version}.${Options.getOption(MQTopics.ENGINE_ADMIN_TOPIC)}`;
	mq.subscribe(engineAdminTopicVersion, (msg) => admin.callback(msg));

	const engineAdminTopic = `*.${Options.getOption(MQTopics.ENGINE_ADMIN_TOPIC)}`;
	mq.subscribe(engineAdminTopic, (msg) => admin.callback(msg));

	/**
	 * Messages received from Bitloops-Engine
	 */
	const srPublishedEventsTopic = `${Options.getOption(MQTopics.WORKFLOW_EVENTS_PREFIX)}.>`;
	mq.subscribe(srPublishedEventsTopic, (msg, subject) => sr.handleNodeIntermediateMessage(msg, subject), engineQueue);

	/**
	 * Commands received from Bitloops-Rest
	 */
	const sseventsTopic = `${Options.getOption(MQTopics.SSE_REGISTER_CONTROL)}.*`;
	mq.subscribe(sseventsTopic, (msg, subject) => sr.registerSubscription(msg, subject), engineQueue);
	// const kpi = nc.subscribe(process.env.ENGINE_KPI_NATS_TOPIC, { queue: process.env.BITLOOPS_ENGINE_QUEUE });
	// const statusBroker = nc.subscribe(process.env.ENGINE_STATUS_LOGGER);
	// @TODO error handling here to the handlers as well as to all the handlers
})();

process.on('exit', async () => {
	console.info('exit');
	await handleShutdown();
});
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
