import { WorkflowParams } from './../../entities/workflow/definitions';
import { replaceVars } from '../../helpers/replaceVariables';
import { IMQ } from '../../services/interfaces';
import { Options } from '../../services';
import { MQTopics } from '../../constants';
import { IMessageTaskNode } from '../../entities/nodes/definitions';
import { MessageResponse, JSONMessageDecodedObject } from './definitions';

class Message {
	private mq: IMQ;

	constructor(mq: IMQ) {
		this.mq = mq;
	}

	async callback(data: JSONMessageDecodedObject): Promise<void> {
		const { nodeDefinition, workflowDefinition, workflowParams } = data;
		const { constants, variables, systemVariables } = workflowParams;
		console.log(`Making ${nodeDefinition.name} Message request for ${variables.instanceId}`);
		let messageResponse = await this.requestMessage(nodeDefinition, workflowParams);

		if (messageResponse.error) {
			// console.log(`Received error in ${node.name} rest response for ${variables.instanceId}`);
			console.error(variables.instanceId, messageResponse.error);
			const varObject = {
				...variables,
				error: messageResponse.error,
			};
		} else {
			// console.log(`Received ${node.name} valid rest response for ${variables.instanceId}`);
			const replaceVarsParams = {
				variables,
				output: messageResponse.value,
				workflowDefinition,
				constants,
				systemVariables,
			};
			const output = await replaceVars(nodeDefinition.type.output, replaceVarsParams);

			if (!systemVariables.nodes[nodeDefinition.id]) systemVariables.nodes[nodeDefinition.id] = {};
			systemVariables.nodes[nodeDefinition.id].executed = true;

			const version = Options.getVersion();
			this.mq.publish(`${version}.${Options.getOption(MQTopics.ENGINE_TOPIC)}`, {
				nodeDefinition,
				workflowParams: { constants, variables: { ...variables, ...output }, systemVariables },
				workflowDefinition,
			});
		}
		messageResponse = null;
	}

	async requestMessage(nodeDefinition: IMessageTaskNode, workflowParams: WorkflowParams): Promise<MessageResponse> {
		console.log('MESSAGE REQUESTING');
		const { topic, message } = nodeDefinition.type.parameters.interface;

		try {
			const parsedMessage = await replaceVars(message, workflowParams);
			const data = await this.mq.request(topic, parsedMessage);

			console.log('MESSAGE RESPONSE OK', data);
			return { value: data, error: null };
		} catch (error) {
			console.error('MESSAGE RESPONSE NOT OK', error);
			return { value: null, error };
		}
	}
}

export default Message;
