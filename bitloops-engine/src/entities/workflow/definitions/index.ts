import { WorkflowContext } from './../../../handlers/bitloopsEngine/definitions/index';
import { INode, ServicesEnum } from '../../nodes/definitions';
import { IServices } from '../../../services/definitions';
import { AuthData } from '../../../handlers/bitloopsEngine/definitions';

export enum variableTypes {
	bool = 'bool',
	string = 'string',
	bytes = 'bytes',
	Struct = 'Struct',
	double = 'double',
	float = 'float',
	int32 = 'int32',
	int64 = 'int64',
	uint32 = 'uint32',
	uint64 = 'uint64',
	sint32 = 'sint32',
	sint64 = 'sint64',
	fixed32 = 'fixed32',
	fixed64 = 'fixed64',
	sfixed32 = 'sfixed32',
	sfixed64 = 'sfixed64',
}

export interface ITypedVariable {
	name: string;
	type: variableTypes;
	evalValue: string;
}

export interface ITypedStringVariable extends ITypedVariable {
	type: variableTypes.string;
}

export interface IBitloopsInputConstant {
	name: string;
	type: string;
	value: string;
}

export interface IRequiredTypedVariable extends ITypedVariable {
	required: boolean;
}

//TODO change it to unknown, if possible
export type BitloopsVariables = Record<string, any>;

export type WorkflowParams = {
	payload?: BitloopsVariables;
	constants?: BitloopsVariables;
	variables?: BitloopsVariables;
	systemVariables?: BitloopsVariables;
	context: WorkflowContext;
};

export interface IEdge {
	from: string;
	to: IToEdge[];
}

export interface IToEdge {
	label?: string;
	nodeId: string;
}

export interface IBitloopsWorkflowDefinition {
	id: string;
	name: string;
	workspaceId: string;
	version: string;
	bitloopsEngineVersion: string;
	debugId: string;
	constants: Record<string, IBitloopsInputConstant[]>;
	nodes: INode[];
	edges: IEdge[];
}

export type WorkflowConstructorArgs = {
	workflowDefinition: IBitloopsWorkflowDefinition;
	services: IServices;
	payload?: any;
	originalReply?: string;
	environmentId: string;
	authData?: AuthData;
	context: WorkflowContext;
};

export type EventTriggerWorkflowInfo = {
	workflowId: string;
	nodeId: string;
	workflowVersion?: string;
	environmentId?: string;
};

export type WorkflowArrayResponse = {
	workflows: EventTriggerWorkflowInfo[];
	error: Error;
};

export type WorkspaceSecretsInfo = {
	id: string;
	name: string;
	workspaceId: string;
	environments: Record<string, secretEnvironment>;
};

export type WorkspaceServicesInfo = {
	id: string;
	name: string;
	description: string;
	tags: string[];
	environments: Record<string, serviceEnvironment>;
} & WorkspaceServiceTypeInfo;

type WorkspaceServiceTypeInfo =
	| WorkspaceGrpcServiceInfo
	| WorkspaceRestServiceInfo
	| WorkspaceMessageServiceInfo
	| WorkspaceDynamicRestServiceInfo;
export type WorkspaceGrpcServiceInfo = {
	type: ServicesEnum.GRPC;
	meta: { proto: string };
};

export type WorkspaceRestServiceInfo = {
	type: ServicesEnum.REST;
	meta: { swagger?: string };
};

export type WorkspaceMessageServiceInfo = {
	type: ServicesEnum.MESSAGE;
	meta: { proto?: string };
};

export type WorkspaceDynamicRestServiceInfo = {
	type: ServicesEnum.DYNAMIC_REST;
};

type serviceEnvironment = {
	target: string;
	ssl: boolean;
};

type secretEnvironment = {
	secretValue: string;
};
