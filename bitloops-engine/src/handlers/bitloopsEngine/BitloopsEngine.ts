import { KEYCLOAK_PK, MQTopics, NOT_VALID_AUTH_MESSAGE } from '../../constants';
import {
	JSONDecodedObject,
	RequestEventMessage,
	PublishEventMessage,
	WorkflowMainInfo,
	WorfklowArgs,
	AuthTypes,
	AuthorizeMessageResponse,
	MessageContext,
	WorkflowContext,
	AuthData,
} from './definitions';
import { Options } from '../../services';
import Workflow from '../../entities/workflow/Workflow';
import { IServices } from '../../services/definitions';
import {
	IBitloopsWorkflowDefinition,
	EventTriggerWorkflowInfo,
	WorkflowArrayResponse,
	WorkflowParams,
} from '../../entities/workflow/definitions';
import { WorkflowDefinition } from '../../entities/workflow/WorkflowDefinition';
import { INode } from '../../entities/nodes/definitions';
import { isJWTExpired, isJWTInvalid, parseJWT } from '../../utils/auth';

export default class BitloopsEngine {
	private services: IServices;

	constructor(services: IServices) {
		this.services = services;
	}

	async handleEventsTopic(message: PublishEventMessage | RequestEventMessage): Promise<void> {
		// console.log('Events topic context:', message.context);
		const authorizeMessageResponse = authorizeMessageContext(message.context);
		console.log('authresponnse', authorizeMessageResponse);
		let requestReply = (message as RequestEventMessage).originalReply;
		if (authorizeMessageResponse.isAuthorized) {
			const { auth: authData } = authorizeMessageResponse;
			const workflowContext: WorkflowContext = {
				request: message.context.request,
				auth: authData?.user,
			};

			const reply = requestReply;
			if (reply) {
				// console.log(message);
				const { workflowId, nodeId, workflowVersion, environmentId, payload, originalReply } =
					message as RequestEventMessage;
				const workflowMainInfo = { workflowId, workflowVersion, environmentId };
				const workflowArgs = {
					payload,
					originalReply,
					environmentId,
					nodeId,
					authData,
					context: workflowContext,
				};
				return this.getWorkflowAndPublishEvent(workflowMainInfo, workflowArgs);
			}
			const { workspaceId, messageId, payload } = message as PublishEventMessage;
			const workflowEventTriggers = await this.getWorkflowEventTriggers({ workspaceId, messageId });
			// TODO (later) improve error handling when we create an appropriate error handling mechanism
			if (workflowEventTriggers.error) {
				console.error(workflowEventTriggers.error);
			} else {
				// TODO check - remove await
				await Promise.all(
					workflowEventTriggers.workflows.map((workflow) => {
						console.log('EventTrigger entity', workflow);
						const { workflowId, nodeId, workflowVersion, environmentId } = workflow;
						const workflowMainInfo = { workflowId, workflowVersion, environmentId };
						const workflowArgs = {
							payload,
							environmentId,
							nodeId,
							authData,
							context: workflowContext,
						};
						return this.getWorkflowAndPublishEvent(workflowMainInfo, workflowArgs);
					}),
				);
			}
		} else {
			console.log('Not valid auth, event rejected');
			if (requestReply) {
				this.services.mq.publish(requestReply, { error: NOT_VALID_AUTH_MESSAGE });
			}
		}
	}

	async handleEngineTopic(message: JSONDecodedObject): Promise<void> {
		const {
			nodeDefinition,
			workflowDefinition,
			payload,
			originalReply,
			workflowParams,
			environmentId,
			authData,
			context,
		} = message;
		console.log('context:', context, workflowParams?.context);

		const workflowConstructorArgs = {
			workflowDefinition,
			services: this.services,
			payload,
			originalReply,
			environmentId,
			authData,
			context,
		};
		const workflow = new Workflow(workflowConstructorArgs);
		if (!this.isWorkflowCreatedFromInitialNode(workflowParams)) {
			workflow.setParams(workflowParams);
		}
		await workflow.handleNode(workflow.getNode(nodeDefinition.id));
	}

	private async getWorkflowEventTriggers({ workspaceId, messageId }): Promise<WorkflowArrayResponse> {
		const { workflowEventTriggerCache, db } = this.services;
		try {
			let workflows = await workflowEventTriggerCache.fetch(workspaceId, messageId);
			// console.log('event triggers cache content ');
			// workflowEventTriggerCache.getSnapshot();
			if (!workflows) {
				// console.log("Didn't find it in redis, looking in Mongo");
				workflows = await db.getWorkflowsTriggeredByEvent(workspaceId, messageId);
				if (workflows === null) {
					throw new Error(`Could not find workflows for ${workspaceId}:${messageId}`);
				}
				workflowEventTriggerCache.cache(workspaceId, messageId, workflows);
			}
			const response = {
				workflows,
				error: null,
			};
			return response;
		} catch (error) {
			const errorResponse = {
				workflows: null,
				error: error,
			};
			return errorResponse;
		}
	}

	private async getWorkflowDefinition(workflowMainInfo: WorkflowMainInfo): Promise<IBitloopsWorkflowDefinition> {
		return WorkflowDefinition.get(workflowMainInfo);
	}

	private isWorkflowCreatedFromInitialNode = (params: WorkflowParams) => {
		return params === undefined || params === null;
	};

	private async getWorkflowAndPublishEvent(workflowMainInfo: WorkflowMainInfo, workflowArgs: WorfklowArgs) {
		const blsWorkflowDefinition: IBitloopsWorkflowDefinition = await this.getWorkflowDefinition(workflowMainInfo);
		const { bitloopsEngineVersion } = blsWorkflowDefinition;
		// TODO feature (later) check input variables to verify correct type and presence of required variables
		// and reply with rejection if not as expected
		// TODO add authentication check
		const engineTopic = `${bitloopsEngineVersion}.${Options.getOption(MQTopics.ENGINE_TOPIC)}`;
		this.services.mq.publish(engineTopic, {
			nodeDefinition: this.getStartNodeDefinition(blsWorkflowDefinition, workflowArgs.nodeId),
			workflowDefinition: blsWorkflowDefinition,
			...workflowArgs,
		});
	}

	private getStartNodeDefinition(blsWorkflowDefinition: IBitloopsWorkflowDefinition, nodeId: string): INode {
		const { nodes } = blsWorkflowDefinition;
		for (let i = 0; i < nodes.length; i++) {
			if (nodes[i].id === nodeId) return nodes[i];
		}
	}
}
function authorizeMessageContext(context: MessageContext): AuthorizeMessageResponse {
	if (!context.auth) return { isAuthorized: false };

	const { authType, authData } = context.auth;

	switch (authType.toLocaleLowerCase()) {
		case AuthTypes.User.toLocaleLowerCase():
			const base64PK = Options.getOption(KEYCLOAK_PK);
			const publicKeyString = Buffer.from(base64PK, 'base64').toString();
			const token = authData;
			if (!authData)
				return {
					isAuthorized: false,
				};
			const jwt = parseJWT(token, publicKeyString);
			// console.log('jwt.JWTData', jwt.jwtData);
			if (isJWTExpired(jwt) || isJWTInvalid(jwt))
				return {
					isAuthorized: false,
				};
			const auth: AuthData = {
				user: {
					id: jwt.jwtData.sub,
					...jwt.jwtData,
				},
			};

			return {
				isAuthorized: true,
				auth,
			};
		case AuthTypes.Anonymous.toLocaleLowerCase():
			return {
				isAuthorized: true,
			};
		case AuthTypes.X_API_KEY.toLocaleLowerCase():
			return {
				isAuthorized: true,
			};
		case AuthTypes.Unauthorized.toLocaleLowerCase():
			return {
				isAuthorized: true,
			};
		case AuthTypes.FirebaseUser.toLocaleLowerCase():
			return {
				isAuthorized: true,
			};
		default:
			return {
				isAuthorized: false,
			};
	}
}
