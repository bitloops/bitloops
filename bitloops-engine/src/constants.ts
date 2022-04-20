export enum MQTopics {
	ENGINE_TOPIC = 'ENGINE_TOPIC',
	ENGINE_ADMIN_TOPIC = 'ENGINE_ADMIN_TOPIC',
	ENGINE_EVENTS_TOPIC = 'ENGINE_EVENTS_TOPIC',
	ENGINE_GRPC_TOPIC = 'ENGINE_GRPC_TOPIC',
	ENGINE_REST_TOPIC = 'ENGINE_REST_TOPIC',
	BITLOOPS_ENGINE_QUEUE = 'BITLOOPS_ENGINE_QUEUE',
	ENGINE_LOGGER_TOPIC = 'ENGINE_LOGGER_TOPIC',
	VERSION = 'VERSION',
	ENGINE_MESSAGE_TOPIC = 'ENGINE_MESSAGE_TOPIC',
}

export enum WorkflowSettings {
	MAX_WORKFLOWS_CACHE = 'MAX_WORKFLOWS_CACHE',
	MAX_EVENT_TRIGGERS_CACHE = 'MAX_EVENT_TRIGGERS_CACHE',
	MAX_SECRET_CACHE = 'MAX_SECRET_CACHE',
	MAX_WORKSPACE_SERVICES_CACHE = 'MAX_WORKSPACE_SERVICES_CACHE',
	MAX_WORKSPACE_SECRETS_CACHE = 'MAX_WORKSPACE_SECRETS_CACHE',
}

export enum ServerSettings {
	SERVICE_PORT = 'SERVICE_PORT',
	DEFAULT_SERVER_PORT = 8080,
}

export enum RedisSettings {
	REDIS_HOST = 'REDIS_HOST',
	REDIS_PORT = 'REDIS_PORT',
	REDIS_PASSWORD = 'REDIS_PASSWORD',
	REDIS_USERNAME = 'REDIS_USERNAME',
	NODE1_ENDPOINT = 'NODE1_ENDPOINT',
	NODE2_ENDPOINT = 'NODE2_ENDPOINT',
}

export const CLOUD_PROVIDER = 'CLOUD_PROVIDER';

export const ENCRYPTION_KEY = 'ENCRYPTION_KEY';

export const KEYCLOAK_PK = 'KEYCLOAK_PK';


export const ADMIN_WORKSPACE_ID = 'ADMIN_WORKSPACE_ID';

export const NOT_VALID_AUTH_MESSAGE = 'Provided authentication was not valid';

export const ADMIN_COMMANDS = {
	GC: 'gc',
	SET_OPTION: 'setOption',
	CLEAR_WORKFLOW_CACHE: 'clearWorkflowCache',
	UPDATE_WORFKLOW_CACHE: 'updateWorkflowCache',
	UPDATE_WORFKLOW_VERSION_MAPPING_CACHE: 'updateWorkflowVersionMappingCache',
	PUBLISH_TO_TOPIC: 'publishToTopic',
}
