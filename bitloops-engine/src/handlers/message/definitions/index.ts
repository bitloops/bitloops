import { IMessageTaskNode } from '../../../entities/nodes/definitions';
import { WorkflowParams, IBitloopsWorkflowDefinition } from '../../../entities/workflow/definitions';

export type MessageResponse = {
	value: any;
	error: Error;
}

export type JSONMessageDecodedObject = {
	nodeDefinition: IMessageTaskNode;
	workflowParams: WorkflowParams;
	workflowDefinition: IBitloopsWorkflowDefinition;
};