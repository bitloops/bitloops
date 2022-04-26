import { NatsConnection } from 'nats';

import { FirebaseCredentials, IXApiKeyDefinition, tokenInfo } from '../definitions';
import FirebaseAdmin from '../FirebaseAdmin';
import { CachedPublicKey } from '../Cache/PublicKeysCache';

export interface IIMDB {
	initializeConnection(): Promise<void>;
	getConnection(): Promise<any>;
	closeConnection(): Promise<void>;
	setSessionInfo(
		sessionState: string,
		sessionInfo: {
			sessionUuid: string;
			providerId: string;
			clientId: string;
			workspaceId: string;
		},
	): Promise<void>;
	getSessionInfo(sessionState: string): Promise<{
		sessionUuid: string;
		providerId: string;
		clientId: string;
		workspaceId: string;
	}>;
}

export interface ILRUCache<T> {
	get(key: string): T;
	set(key: string, value: T): void;
}

export interface IXApiKeyCache {
	cache(xApiKey: string, xApiKeyRecord: IXApiKeyDefinition); // imdb.setHashValueObject(`blsw:${workflowId}`, workflowVersion.toString(), workflow as any);
	fetch(xApiKey: string): Promise<IXApiKeyDefinition>;
}

export interface ISecretCache {
	cache(workflowId: string, workflowVersion: string, secrets: any);
	fetch(workflowId: string, workflowVersion: string): any; // .getHashValues(`blsws:${workflowId}`)
}

export interface IPublicKeysCache {
	cache(providerId: string, pk: string);
	fetch(providerId: string): CachedPublicKey;
}

export interface IWorkflowEventTriggerCache {
	cache(workspaceId: string, messageId: string, workflowId: string): Promise<void>; // imdb.setHashValueObject(`blset:${workspaceId}:${messageId}`, workflowId, value),
	fetch(workspaceId: string, messageId: string): Promise<string[]>;
}

export interface IRunningRequestsCache {
	getCount(): Promise<number>;
	delete(instanceId: string): Promise<void>;
	cache(instanceId: string): Promise<void>;
}

export interface IFirebaseConnectionsCache {
	cache(connectionId: string, connection: FirebaseAdmin);
	fetch(connectionId: string): FirebaseAdmin;
}

export interface IFirebaseTokensCache {
	cache(token: string, tokenInfo: Omit<tokenInfo, 'cached_at'>);
	fetch(token: string): tokenInfo;
}

export interface ILogger {
	log(data: Record<string, unknown>): Promise<void>;
}

export interface IMQ {
	initializeConnection(): Promise<NatsConnection>;
	getConnection(): Promise<NatsConnection>;
	closeConnection(): Promise<void>;
	gracefullyCloseConnection(): Promise<void>;
	publish(topic: string, message: Record<string, unknown> | string): Promise<void>;
	request<T>(topic: string, body: any): Promise<T>;
	subscribe(topic: string, callbackFunction?: (data: any, subject: string) => void, subscriptionGroup?: string): void;
}

export interface IDatabase {
	connect(): Promise<void>;
	disconnect(): Promise<void>;
	getWorkflowsTriggeredByEvent(
		workspaceId: string,
		messageId: string,
	): Promise<Array<{ workflowId: string; version?: number }> | null>;
	getXApiKey(xApiKey: string): Promise<IXApiKeyDefinition | null>;
	getSecrets(workflowId: string, version: number): Promise<any>;
	getFirebaseCredentials(providerId: string): Promise<FirebaseCredentials | null>;
	getProviderClientSecret(providerId: string, clientId: string): Promise<string>;
}

// export interface IWorkflow {
// 	getServices(): Services;
// 	getNextNode(nodeId: string, variables: BitloopsVariables): BaseNode;
// 	getWorkflow(): IBitloopsWorkflowDefinition;
// 	getParams(): WorkflowParams;
// 	getTriggerPayload(): Record<string, unknown>;
// 	setVariablesParams(variables: BitloopsVariables): void;
// 	getToEdges(nodeId: string): IToEdge[] | null;
// 	getNodeDefinition(nodeId: string): INode;
// 	getNodes(): Record<string, BaseNode>;
// 	getNode(nodeId: string): BaseNode;
// }
