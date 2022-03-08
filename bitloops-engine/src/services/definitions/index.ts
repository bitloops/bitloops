import {
	IRunningWorkflowInstanceCache,
	ISecretCache,
	IWorkflowCache,
	IWorkflowEventTriggerCache,
	IDatabase,
	IMQ,
	IIMDB,
	ILogger,
	IWorkspaceServicesCache,
	IWorkspaceSecretsCache,
} from '../interfaces';

export interface IServices {
	runningWorkflowInstanceCache: IRunningWorkflowInstanceCache;
	secretCache: ISecretCache;
	workflowCache: IWorkflowCache;
	workflowEventTriggerCache: IWorkflowEventTriggerCache;
	workspaceServicesCache: IWorkspaceServicesCache;
	workspaceSecretsCache: IWorkspaceSecretsCache;
	db: IDatabase;
	mq: IMQ;
	imdb: IIMDB;
	logger: ILogger;
	Options?: any; // TODO make Options singleton?;
}

export type MQSubscriptionCallbackFunc = (data: any, subject?: string) => void;
