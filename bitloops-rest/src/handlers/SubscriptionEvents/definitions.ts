import { SSE_MESSAGE_TYPE } from '../../constants';

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

interface NatsReplyRequest {
	originalReply?: string;
}
