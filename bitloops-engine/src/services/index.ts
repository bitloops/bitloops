import { v4 as uuid } from 'uuid';

import NATS from './NATS';
import {
	RunningWorkflowInstanceCache,
	SecretCache,
	WorkflowCache,
	WorkflowEventTriggerCache,
	WorkspaceServicesCache,
	WorkspaceSecretsCache,
} from './Cache';
import Options from './Options';
import Mongo from './Mongo';
import Logger from './Logger';
import {
	IMQ,
	IDatabase,
	ILogger,
	IRunningWorkflowInstanceCache,
	ISecretCache,
	IWorkflowCache,
	IWorkflowEventTriggerCache,
	IWorkspaceServicesCache,
	IWorkspaceSecretsCache,
	IIMDB,
} from './interfaces';
import { WorkflowSettings } from '../constants';
import { IServices } from '../services/definitions';
import Redis from './Redis';

class Services {
	private static runningWorkflowInstanceCache: IRunningWorkflowInstanceCache = new RunningWorkflowInstanceCache();
	private static secretCache: ISecretCache = new SecretCache(
		Options.getOptionAsNumber(
			WorkflowSettings.MAX_SECRET_CACHE,
			Options.getOptionAsNumber(WorkflowSettings.MAX_WORKFLOWS_CACHE, 1000),
		),
	);
	private static workflowCache: IWorkflowCache = new WorkflowCache(
		Options.getOptionAsNumber(WorkflowSettings.MAX_WORKFLOWS_CACHE, 1000),
	);
	private static workflowEventTriggerCache: IWorkflowEventTriggerCache = new WorkflowEventTriggerCache(
		Options.getOptionAsNumber(WorkflowSettings.MAX_EVENT_TRIGGERS_CACHE, 1000),
	);
	private static workspaceServicesCache: IWorkspaceServicesCache = new WorkspaceServicesCache(
		Options.getOptionAsNumber(WorkflowSettings.MAX_WORKSPACE_SERVICES_CACHE, 1000),
	);
	private static workspaceSecretsCache: IWorkspaceSecretsCache = new WorkspaceSecretsCache(
		Options.getOptionAsNumber(WorkflowSettings.MAX_WORKSPACE_SECRETS_CACHE, 1000),
	);
	private static mq: IMQ = new NATS(Options.getOption('NATS_IP') ? undefined : {});
	private static db: IDatabase = new Mongo();
	private static imdb: IIMDB = new Redis();
	private static logger: ILogger = new Logger(Services.mq);
	private static services: IServices;

	static async initializeServices(): Promise<IServices> {
		console.info('Initializing DB...');
		await Services.db.connect();
		Options.setOption('dbReady', 'true');
		console.info('Connected to DB!');
		console.info('Initializing imdb...');
		await Services.imdb.initializeConnection();
		Options.setOption('imdbReady', 'true');
		console.info('Connected to IMDB!');
		console.info('Initializing MQ...');
		await Services.mq.getConnection();
		Options.setOption('mqReady', 'true');
		console.info('Connected to MQ!');
		Options.setServerUUID(uuid());
		const services = {
			db: Services.db,
			logger: Services.logger,
			mq: Services.mq,
			imdb: Services.imdb,
			runningWorkflowInstanceCache: Services.runningWorkflowInstanceCache,
			secretCache: Services.secretCache,
			workflowCache: Services.workflowCache,
			workflowEventTriggerCache: Services.workflowEventTriggerCache,
			workspaceServicesCache: Services.workspaceServicesCache,
			workspaceSecretsCache: Services.workspaceSecretsCache,
			Options: Options,
		};
		Services.services = services;
		return services;
	}

	static getServices(): IServices {
		return Services.services;
	}
}

export default Services;
export { Options };
