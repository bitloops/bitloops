import { v4 as uuid } from 'uuid';

export const statusHandler = async (sub, jc) => {
	for await (const m of sub) {
		const { node, variables } = jc.decode(m.data);
	}
};
