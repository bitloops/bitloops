import { IGrpcTaskNode } from '../../../entities/nodes/definitions';
import { WorkflowParams, IBitloopsWorkflowDefinition } from '../../../entities/workflow/definitions';

export interface IGRPCResponse {
	value: any;
	error: Error;
}

export type JSONGrpcDecodedObject = {
	nodeDefinition: IGrpcTaskNode;
	workflowParams: WorkflowParams;
	workflowDefinition: IBitloopsWorkflowDefinition;
};