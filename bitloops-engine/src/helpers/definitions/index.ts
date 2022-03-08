import { BitloopsVariables } from "../../entities/workflow/definitions";

export type ReplaceVarsParams = {
	payload?: BitloopsVariables,
	secrets?: BitloopsVariables,
	variables?: BitloopsVariables,
	systemVariables?: BitloopsVariables,
	constants?: BitloopsVariables,
	output?: BitloopsVariables,
}