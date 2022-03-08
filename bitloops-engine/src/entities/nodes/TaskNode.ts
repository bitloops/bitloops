import { BaseNode } from './BaseNode';
import { MQTopics } from '../../constants';
import { WorkflowParams } from '../workflow/definitions';
import { ITaskNode, ServicesEnum } from './definitions';

class TaskNode extends BaseNode {
	async execute(workflowParams: WorkflowParams): Promise<WorkflowParams> {
		this.startedAt = Date.now();
		console.log('TASK NODE ID-->', this.id);
		const { systemVariables } = workflowParams;
		const { mq, logger, Options } = this.services;
		const executed = systemVariables?.nodes[this.id]?.executed;
		// console.log('Executed ', executed);
		if (!executed) {
			// TODO How to distinguish grpcTaskNode from other task nodes
			const version = Options.getVersion();
			const nodeDefinition = this.nodeDefinition as ITaskNode;
			// console.log('interface.type', nodeDefinition.type);
			if (nodeDefinition.type.parameters.interface.type === ServicesEnum.GRPC) {
				// TODO maybe catch the error here in publish
				const engineGRPCTopic = `${version}.${Options.getOption(MQTopics.ENGINE_GRPC_TOPIC)}`;
				mq.publish(engineGRPCTopic, {
					nodeDefinition,
					workflowParams,
					workflowDefinition: this.workflow.getWorkflow(),
				});
				return null;
			} else if (nodeDefinition.type.parameters.interface.type === ServicesEnum.REST || nodeDefinition.type.parameters.interface.type === ServicesEnum.DYNAMIC_REST) {
				const engineRESTTopic = `${version}.${Options.getOption(MQTopics.ENGINE_REST_TOPIC)}`;
				mq.publish(engineRESTTopic, {
					nodeDefinition,
					workflowParams,
					workflowDefinition: this.workflow.getWorkflow(),
				});
				return workflowParams;
			} else if (nodeDefinition.type.parameters.interface.type === ServicesEnum.MESSAGE) {
				const engineMessageTopic = `${version}.${Options.getOption(MQTopics.ENGINE_MESSAGE_TOPIC)}`;
				mq.publish(engineMessageTopic, {
					nodeDefinition,
					workflowParams,
					workflowDefinition: this.workflow.getWorkflow(),
				});
				return workflowParams;
			}
		} else {
			return super.execute(workflowParams);
		}
	}

	async getNext(): Promise<BaseNode> {
		const executed = this.workflow.getParams()?.systemVariables?.nodes[this.id]?.executed;
		if (!executed) {
			return null;
		} else {
			return super.getNext();
		}
	}
}

export default TaskNode;
