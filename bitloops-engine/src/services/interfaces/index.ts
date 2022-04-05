import { BaseNode } from '../../entities/nodes';
import {
	BitloopsVariables,
	IBitloopsWorkflowDefinition,
	WorkflowParams,
	IToEdge,
	EventTriggerWorkflowInfo,
	WorkspaceServicesInfo,
	WorkspaceSecretsInfo,
} from '../../entities/workflow/definitions';
import { INode } from '../../entities/nodes/definitions';
import { MQSubscriptionCallbackFunc, IServices } from '../definitions';

export interface ITopicIdInfo {
	workspaceId: string;
	topic: string;
}

export interface ITopicValueInfo {
	workspaceId: string;
	topics: string[];
}

export interface IIMDB {
	initializeConnection(): Promise<void>;
	getConnection(): Promise<any>;
	closeConnection(): Promise<void>;
	addConnectionIdToTopic(idInfo: ITopicIdInfo, connectionId: string): Promise<void>;
	getConnectionIdsSubscribedToTopic(workspaceId: string, topic: string): Promise<string[]>;
	addTopicsToConnectionId(connectionId: string, valueInfo: ITopicValueInfo): Promise<void>;
	getConnectionIdValue(connectionId: string): Promise<{ [key: string]: string }>;
	/** Stores new values in connectionId hashSet
	 * matched existing fields in the hash are overwritten */
	storeConnectionIdValue(connectionId: string, value: Record<string, string>): Promise<void>;
	removeConnectionId(connectionId: string): Promise<void>;
	/** Links new connection to PodId */
	addConnectionToPodId(podId: string, connectionId: string): Promise<void>;
	/** Removes state of dead pod */
	cleanPodState(podId: string): Promise<void>;
	/**
	 * Gets called when client unsubscribes from a topic(namedEvent)
	 * Has to remove the connectionId from the topicToConnections mapping, 
	 * and its reverse mapping 
	 * 
	 * @param connectionId string
	 * @param workspaceId string
	 * @param topic string
	 * 
	 * @not response for killing-removing the entire connection
	 */
	handleTopicUnsubscribe(connectionId: string, workspaceId: string, topic: string): Promise<void>;
}

export interface ILRUCache<T> {
	get(key: string): T;
	set(key: string, value: T): void;
	remove(key: string): void;
	getCount(): number;
	getSnapshot(): void;
}

export interface IWorkflowCache extends ILRUCache<Record<string, IBitloopsWorkflowDefinition>> {
	cache(workflowId: string, workflowVersion: string, environmentId: string, workflow: IBitloopsWorkflowDefinition); // imdb.setHashValueObject(`blsw:${workflowId}`, workflowVersion.toString(), workflow as any);
	fetch(workflowId: string, workflowVersion: string, environmentId: string): Promise<IBitloopsWorkflowDefinition | null>;
	delete(workflowId: string, workflowVersion: string);
}

export interface IWorkflowVersionMappingCache extends ILRUCache<string> {
	cache(workflowId: string, workflowVersion: string);
	fetch(workflowId: string): Promise<string>;
	delete(workflowId: string);
}

export interface ISecretCache extends ILRUCache<any> {
	cache(workflowId: string, workflowVersion: string, secrets: any);
	fetch(workflowId: string, workflowVersion: string): any; // .getHashValues(`blsws:${workflowId}`)
}
// export type eventTrigger = { workflowId: string; workflowVersion?: number };
export interface IWorkflowEventTriggerCache extends ILRUCache<EventTriggerWorkflowInfo[]> {
	cache(workspaceId: string, messageId: string, value: EventTriggerWorkflowInfo[]): Promise<void>; // imdb.setHashValueObject(`blset:${workspaceId}:${messageId}`, workflowId, value),
	fetch(workspaceId: string, messageId: string): Promise<EventTriggerWorkflowInfo[]>;
}

export interface IWorkspaceServicesCache extends ILRUCache<WorkspaceServicesInfo> {
	cache(workspaceId: string, serviceId: string, data: WorkspaceServicesInfo): Promise<void>;
	cacheServices(workspaceId: string, services: Record<string, WorkspaceServicesInfo>): Promise<void>;
	fetchServices(workspaceId: string, services: Set<string>): Promise<Record<string, WorkspaceServicesInfo>>;
}

export interface IWorkspaceSecretsCache extends ILRUCache<WorkspaceSecretsInfo> {
	cache(workspaceId: string, serviceId: string, data: WorkspaceSecretsInfo): Promise<void>;
	deleteSecret(workspaceId: string, secretId: string): Promise<void>
	cacheSecrets(workspaceId: string, secrets: Record<string, WorkspaceSecretsInfo>): Promise<void>;
	fetchSecrets(workspaceId: string, secretIds: string[]): Promise<Record<string, WorkspaceSecretsInfo>>;
}

export interface IRunningWorkflowInstanceCache {
	getCount(): Promise<number>;
	delete(instanceId: string): Promise<void>;
	cache(instanceId: string): Promise<void>;
}

export interface ILogger {
	log(data: Record<string, unknown>): Promise<void>;
}

export interface IMQ {
	getConnection(): Promise<any>;
	closeConnection(): Promise<void>;
	gracefullyCloseConnection(): Promise<void>;
	request<T>(topic: string, body: any): Promise<T>;
	publish(topic: string, message: Record<string, unknown> | string): Promise<void>;
	subscribe(topic: string, callbackFunction?: MQSubscriptionCallbackFunc, subscriptionGroup?: string): void;
}

export interface IDatabase {
	connect(): Promise<void>;
	disconnect(): Promise<void>;
	getWorkflowsTriggeredByEvent(workspaceId: string, messageId: string): Promise<EventTriggerWorkflowInfo[] | null>;
	getWorkflow(workflowId: string, version?: string): Promise<IBitloopsWorkflowDefinition | null>;
	getWorkflowServices(
		workspaceId: string,
		services: string[],
		environmentId: string,
	): Promise<Record<string, WorkspaceServicesInfo>>;
	getSecretsById(
		workspaceId: string,
		secretIds: string[],
	): Promise<Record<string, WorkspaceSecretsInfo>>;
}

export interface IWorkflow {
	getServices(): IServices;
	getNextNode(nodeId: string, variables: BitloopsVariables): BaseNode;
	getWorkflow(): IBitloopsWorkflowDefinition;
	getParams(): WorkflowParams;
	getStartPayload(): Record<string, unknown>;
	setVariablesParams(variables: BitloopsVariables): void;
	getToEdges(nodeId: string): IToEdge[] | null;
	getNodeDefinition(nodeId: string): INode;
	getNodes(): Record<string, BaseNode>;
	getNode(nodeId: string): BaseNode;
}
