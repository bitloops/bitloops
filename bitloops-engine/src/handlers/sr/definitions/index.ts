import { BitloopsVariables } from '../../../entities/workflow/definitions/index';
export interface ISRResponse {
	value: any;
	error: Error;
}

export type SubscriptionRouterArgs = {
	// workspaceId: string;
	// topic: string;
	payload: BitloopsVariables;

	// nodeDefinition: IGrpcTaskNode;
	// workflowParams: WorkflowParams;
	// workflowDefinition: IBitloopsWorkflowDefinition;
};
export enum SSE_MESSAGE_TYPE {
	VALIDATION = 'validation',
	TOPICS_ADD_CONNECTION = 'topics-add-connection',
	POD_ID_REGISTRATION = 'pod-id-registration',
	CONNECTION_END = 'connection-end',
	POD_SHUTDOWN = 'pod-shutdown',
	TOPIC_UNSUBSCRIBE = 'topic-unsubscribe'
}

export interface ISSEMessage extends NatsReplyRequest {
	type: IConnectionValidate | ITopicsMapping | IInstanceRegistration | IConnectionEnd | IPodShutdown | IUnsubscribeTopic;
}

interface IConnectionValidate {
	name: SSE_MESSAGE_TYPE.VALIDATION;
}

export interface ITopicsMapping {
	name: SSE_MESSAGE_TYPE.TOPICS_ADD_CONNECTION;
	topics: string[];
	newConnection: boolean;
	workspaceId: string;
	creds?: any;
}

interface IInstanceRegistration {
	name: SSE_MESSAGE_TYPE.POD_ID_REGISTRATION;
	podId: string;
}

interface IConnectionEnd {
	name: SSE_MESSAGE_TYPE.CONNECTION_END;
}

interface IPodShutdown {
	name: SSE_MESSAGE_TYPE.POD_SHUTDOWN;
	podId: string;
}

interface IUnsubscribeTopic {
	name: SSE_MESSAGE_TYPE.TOPIC_UNSUBSCRIBE;
	topic: string;
	workspaceId: string;
}

interface NatsReplyRequest {
	originalReply?: string;
}
