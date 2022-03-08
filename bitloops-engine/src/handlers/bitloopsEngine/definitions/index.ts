import { INode } from '../../../entities/nodes/definitions';
import { WorkflowParams, IBitloopsWorkflowDefinition } from '../../../entities/workflow/definitions';

export type JSONDecodedObject = {
	nodeDefinition: INode;
	payload?: any;
	workflowDefinition: IBitloopsWorkflowDefinition;
	workflowMainInfo?: WorkflowMainInfo;
	workflowParams: WorkflowParams;
	originalReply?: string;
	environmentId: string;
	authData: AuthData;
};

export type WorfklowArgs = {
	payload: any;
	originalReply?: string;
	environmentId: string;
	nodeId: string;
	authData: AuthData
};

export type WorkflowMainInfo = {
	workflowId: string;
	// workspaceId: string;
	workflowVersion: string;
	environmentId: string;
	debugId?: string;
};
export interface PublishEventMessage {
	workspaceId: string;
	messageId: string;
	payload: any;
	context: MessageContext;
}

export interface RequestEventMessage {
	workspaceId: string;
	workflowId: string;
	nodeId: string;
	workflowVersion: string;
	environmentId: string;
	payload: any;
	originalReply: string;
	context: MessageContext;
}
//Context of message consumed
export type MessageContext = {
	authType: AuthTypes,
	authData: AuthContextData
}

export enum AuthTypes {
	Basic = 'Basic',
	X_API_KEY = 'x-api-key',
	Token = 'Token',
	FirebaseUser = 'FirebaseUser',
	User = 'User',
	Anonymous = 'Anonymous',
	Unauthorized = 'Unauthorized',
}

export type AuthorizeMessageResponse = {
	isAuthorized: boolean,
	auth?: AuthData
}

export type AuthData = UserAuthData; //TODO add | xapikeyresponse etc when the form is final

type AuthContextData = UserContextData; //TODO add  xapikey etc

type UserContextData = string; //JWT

type UserAuthData = {
	user: {
		id: string
	}
}
