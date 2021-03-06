import { AuthorizedRequest } from "./routes/definitions";

export const enum AppOptions {
    NATS_IP = 'NATS_IP',
    NATS_PORT = 'NATS_PORT',
    NATS_USER = 'NATS_USER',
    NATS_PASSWORD = 'NATS_PASSWORD',
    MAX_X_API_KEY_CACHE = 'MAX_X_API_KEY_CACHE',
    X_API_KEY_CACHE_TIMEOUT = 'X_API_KEY_CACHE_TIMEOUT',
    UNAUTHORIZED_STATUS = 401,
    AUTHORIZED_STATUS = 200,
    SHA256 = 'sha256',
    SHA256_SALT = 'SHA256_SALT',
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

export const ALLOW_ORIGIN_HEADERS = { [CORS.HEADERS.ACCESS_CONTROL_ALLOW_ORIGIN]: CORS.ALLOW_ORIGIN };

export enum AuthTypes {
    Basic = 'Basic',
    X_API_KEY = 'x-api-key',
    Token = 'Token',
    FirebaseUser = 'FirebaseUser',
    User = 'User',
    Anonymous = 'Anonymous',
    Unauthorized = 'Unauthorized',
}

export enum RequestHeaders {
    WORKFLOW_ID = 'workflow-id',
    NODE_ID = 'node-id',
    ENV_ID = 'environment-id',
    WORKFLOW_VERSION = 'workflow-version',
    WORKSPACE_ID = 'workspace-id',
}

export enum KeycloakSettings {
    PUBLIC_KEY = 'KEYCLOAK_PK',
}

export const ERMIS_CONNECTION_PREFIX_TOPIC = 'ermis.connection';

export const UNAUTHORIZED_REQUEST: AuthorizedRequest = {
    verification: {
        authType: AuthTypes.Unauthorized,
    }
};

export const WORKFLOW_EVENTS_PREFIX: string = 'workflow-events';

export enum ERMIS_CONNECTION_TOPIC_ACTIONS {
    SUBSCRIBE = 'subscribe',
    UNSUBSCRIBE = 'unsubscribe',
}