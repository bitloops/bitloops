import { ITypedVariable, IRequiredTypedVariable, WorkflowParams, ITypedStringVariable } from '../../workflow/definitions';

export enum NodeTypeName {
	RequestStartNode = 'request-start-event',
	MessageStartNode = 'message-start-event',
	DirectStartNode = 'direct-start-event',
	MessageIntermediateNode = 'message-intermediate-event',
	EndNode = 'end-event',
	TaskNode = 'task-activity',
	SubProcessNode = 'sub-process-activity',
	TransactionNode = 'transaction-activity',
	CallNode = 'call-activity', // https://forum.camunda.org/t/difference-between-call-activity-and-subprocess/20153
	ExclusiveNode = 'exclusive-gateway',
	EventBasedNode = 'event-based-gateway',
	ParallelNode = 'parallel-gateway',
	InclusiveNode = 'inclusive-gateway',
	ExclusiveEventBasedNode = 'exclusive-event-based-gateway',
	ComplexNode = 'complex-gateway',
	ParallelEventBasedNode = 'parallel-event-based-gateway',
	TimerIntermediateNode = 'timer-intermediate-event',
}

export enum ServicesEnum {
	GRPC = 'gRPC',
	REST = 'REST',
	NATS = 'NATS',
	MESSAGE = 'MESSAGE',
	DYNAMIC_REST = 'DYNAMIC_REST',
}

export interface IVisual {
	x: number;
	y: number;
	colour: string;
}

export enum AuthenticationType {
	Anonymous = 'Anonymous',
	Token = 'Token',
	User = 'User',
	OAuth2 = 'OAuth2',
	Basic = 'Basic',
	XAPIKey = 'X-API-Key',
}

export type IStartNodeType = IDirectStartNodeType | IMessageStartNodeType | IRequestStartNodeType;

export enum IntermediateMessageKeys {
	ADMIN = 'admin',
	EVENT = 'event',
	WORKFLOW_EVENT = 'workflow-event',
	PUBLISH_EVENT = 'publish-event',
}
export type AdminType = {
	evalTopic: string;
	command: string;
	payload: ITypedVariable[];
};

export type EventType = {
	messageId: string;
	payload: ITypedVariable[];
};

export type PublishEventType = {
	type: string;
	topic: string;
	payload: ITypedVariable[];
};

export type NodeMessageType = {
	[IntermediateMessageKeys.ADMIN]?: AdminType[];
	[IntermediateMessageKeys.EVENT]?: EventType[];
	[IntermediateMessageKeys.PUBLISH_EVENT]?: PublishEventType[];
};

export type SendEventParams = {
	workspaceId: string;
	messages: AdminType[] | PublishEventType[] | EventType[];
	workflowParams: WorkflowParams;
	Options?: any;
};

export interface INode {
	id: string;
	name: string;
	type: IStartNodeType | ITaskNodeType | ICallNodeType | IExclusiveNodeType | IEndNodeType | ITimerNodeType;
	visual: IVisual;
	debugId?: string;
	messages?: NodeMessageType;
}

export type ITimerNodeType = ITimerIntermediateNodeType;

export interface ITimerIntermediateNodeParameters {
	timerDuration: number;
}

export interface ITimerIntermediateNodeType {
	name: NodeTypeName.TimerIntermediateNode;
	parameters: ITimerIntermediateNodeParameters;
}

export interface ITimerNode extends INode {
	type: ITimerNodeType;
}
export interface IExclusiveNode extends INode {
	type: IExclusiveNodeType;
}
export interface IGrpcTaskNode extends INode {
	type: IGrpcTaskNodeType;
}
export interface IRestTaskNode extends INode {
	type: IRestTaskNodeType;
}
export interface IMessageTaskNode extends INode {
	type: IMessageTaskNodeType;
}

export interface ICallNode extends INode {
	type: ICallNodeType;
}

export interface ITaskNode extends INode {
	type: ITaskNodeType;
	executed: boolean;
}
export interface IEndNode extends INode {
	type: IEndNodeType;
}
export interface IStartNode extends INode {
	id: string;
	type: {
		name: NodeTypeName.RequestStartNode;
		alias?: string; // unique id that can be used to trigger the workflow instead of the workflowId
		authentication: IAuthentication;
		input: IRequiredTypedVariable[];
	};
	visual: IVisual;
	debugId?: string;
}

export type NodeHandlerParams = {
	nextNode: Node;
	variables: any;
	secrets: any;
	systemVariables: any;
};

export interface IAuthentication {
	type: AuthenticationType;
	user?: string;
	key?: string;
	Anonymous?: boolean;
	Token?: string;
	OAuth2?: string;
	Basic?: { username: string; password: string };
	'X-API-Key'?: string;
}

export interface IDirectStartNodeType {
	name: NodeTypeName.DirectStartNode;
	workspaceId: string;
	workflowId: string;
	workflowVersion: string;
	input: IRequiredTypedVariable[];
}

export interface IMessageStartNodeType {
	name: NodeTypeName.MessageStartNode;
	entry: string;
	messageId: string;
	authentication: IAuthentication;
	input: IRequiredTypedVariable[];
}

export interface IRequestStartNodeType {
	name: NodeTypeName.RequestStartNode;
	alias?: string;
	authentication: IAuthentication;
	input: IRequiredTypedVariable[];
}

export interface ICallNodeType {
	name: NodeTypeName.CallNode;
	parameters: ICallNodeParameters;
	output?: ITypedVariable[];
}

export interface ICallNodeParameters {
	workflowId: string;
	nodeId: string;
	workflowVersion: string;
	input: ITypedVariable[];
}

export interface ITaskNodeType {
	name: NodeTypeName.TaskNode;
	startedAt?: number;
	parameters: ITaskNodeParameters;
	output?: ITypedVariable[];
}

export interface ITaskNodeParameters {
	interface: IServiceInterfaceType;
	service: string;
	serviceVersion: string;
}

export interface IGrpcTaskInterface extends IServiceInterfaceType {
	input?: ITypedVariable[];
	grpcPackage?: string;
	grpcService: string;
	proto: string;
	rpc: string;
	ssl?: boolean;
	target: string;
}

export enum RestMethodsEnum {
	POST = 'POST',
	GET = 'GET',
	PUT = 'PUT',
	DELETE = 'DELETE',
	PATCH = 'PATCH',
	OPTIONS = 'OPTIONS',
}

export interface IRestTaskInterface extends IServiceInterfaceType {
	// input?: ITypedVariable[];
	method: RestMethodsEnum;
	uri: string;
	urlPath: string; // base URL + URL path
	headers?: ITypedStringVariable[];
	body?: ITypedVariable[];
	query?: ITypedVariable[];
	params?: ITypedStringVariable[];
	swagger?: string;
}

export interface IMessageTaskInterface extends IServiceInterfaceType {
	topic: string;
	message: ITypedVariable[];
	proto?: string;
}

export interface IGrpcTaskParameters extends ITaskNodeParameters {
	interface: IGrpcTaskInterface;
}

export interface IRestTaskParameters extends ITaskNodeParameters {
	interface: IRestTaskInterface;
}

export interface IMessageTaskParameters extends ITaskNodeParameters {
	interface: IMessageTaskInterface;
}

export interface IGrpcTaskNodeType extends ITaskNodeType {
	parameters: IGrpcTaskParameters;
	// service: string; // TODO remove service and serviceVersion since they are inside parameters?
	// serviceVersion: string;
}

export interface IRestTaskNodeType extends ITaskNodeType {
	parameters: IRestTaskParameters;
	// service: string;
	// serviceVersion: string;
}

export interface IMessageTaskNodeType extends ITaskNodeType {
	parameters: IMessageTaskParameters;
	// service: string;
	// serviceVersion: string;
}

export interface IEndNodeType {
	name: NodeTypeName.EndNode;
	output: ITypedVariable[];
}

export interface IExclusiveNodeType {
	name: NodeTypeName.ExclusiveNode;
	parameters: IGatewayTypeParameters;
}

interface IGatewayDefaultCase {
	output?: ITypedVariable[];
	index?: number;
}

export interface IGatewayCase extends IGatewayDefaultCase {
	evalValue: string;
	valueType: string;
	index: number;
}

export interface IGatewayTypeParameters {
	expression: string;
	cases: IGatewayCase[];
	default: IGatewayDefaultCase;
}

export type ServiceType = Record<ServicesEnum, string>;

export interface IServiceInterfaceType {
	type: ServicesEnum; // TODO check ServiceType above
	// target: string;
}

export type NodeLoggerData = {
	workspaceId: string;
	workflowId: string;
	instanceId: string;
	nodeId: string;
	duration?: number;
	debugId?: string;
	occurredAt: number;
	durationSinceStart?: number;
	debugInfo?: string;
};

export type NodeKPIData = {
	workspaceId: string;
	workflowId: string;
	instanceId: string;
	kpi: string;
	value: number;
	debugId?: string;
	occurredAt: number;
};
