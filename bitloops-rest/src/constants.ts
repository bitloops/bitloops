export const enum AppOptions {
	SHA256 = 'sha256',
	SHA256_SALT = 'SHA256_SALT',
	REST_LOGGER_TOPIC = 'REST_LOGGER_TOPIC',
	MAX_X_API_KEY_CACHE = 'MAX_X_API_KEY_CACHE',
	X_API_KEY_CACHE_TIMEOUT = 'X_API_KEY_CACHE_TIMEOUT',
	BITLOOPS_ENGINE_EVENTS_TOPIC = 'BITLOOPS_ENGINE_EVENTS_TOPIC',
	UNAUTHORIZED_STATUS = 401,
	AUTHORIZED_STATUS = 200,
	NATS_IP = 'NATS_IP',
	NATS_PORT = 'NATS_PORT',
	NATS_USER = 'NATS_USER',
	NATS_PASSWORD = 'NATS_PASSWORD',
	BITLOOPS_ENGINE_URL = 'BITLOOPS_ENGINE_URL',
}

export const CORS = {
	HEADERS: {
		ACCESS_CONTROL_ALLOW_ORIGIN: 'Access-Control-Allow-Origin',
		ACCESS_CONTROL_ALLOW_HEADERS: 'Access-Control-Allow-Headers',
	},
	ALLOW_ORIGIN: '*',
	ALLOW_HEADERS:
		'cache-control,authorization,content-type,environment-id,node-id,provider-id,workflow-id,workspace-id,client-id,session-uuid,message-id',
};

export enum RequestType {
	REQUEST = 'request',
	PUBLISH = 'publish',
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

export enum MQTopics {
	WORKFLOW_EVENTS_TOPIC = 'WORKFLOW_EVENTS_TOPIC',
}

export const BITLOOPS_PROVIDER_ID = 'BITLOOPS_PROVIDER_ID';

export enum RequestHeaders {
	WORKFLOW_ID = 'workflow-id',
	NODE_ID = 'node-id',
	ENV_ID = 'environment-id',
	WORKFLOW_VERSION = 'workflow-version',
	WORKSPACE_ID = 'workspace-id',
}

export enum PublishHeaders {
	MESSAGE_ID = 'message-id',
	WORKSPACE_ID = 'workspace-id',
}

export enum RedisSettings {
	REDIS_HOST = 'REDIS_HOST',
	REDIS_PORT = 'REDIS_PORT',
	REDIS_PASSWORD = 'REDIS_PASSWORD',
	REDIS_USERNAME = 'REDIS_USERNAME',
	NODE1_ENDPOINT = 'NODE1_ENDPOINT',
	NODE2_ENDPOINT = 'NODE2_ENDPOINT',
}

export enum KeycloakSettings {
	CLIENT_SECRET = 'KEYCLOAK_CLIENT_SECRET',
	PUBLIC_KEY = 'KEYCLOAK_PK',
	COOKIES_DOMAIN = 'COOKIES_DOMAIN',
}

export const CLOUD_PROVIDER = 'CLOUD_PROVIDER';

export const ERMIS_CONNECTION_PREFIX_TOPIC = 'ermis.connection';
