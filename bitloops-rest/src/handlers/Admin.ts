import { Options } from '../services';

class Admin {
	static callback(data: { command: string; payload: string }): void {
		const { command, payload } = data;
		if (command === 'gc') {
			// console.log('Running gc');
			global.gc();
		} else if (command === 'setOption') {
			try {
				const payloadObject = JSON.parse(payload);
				if (
					payloadObject.key &&
					payloadObject.value &&
					(!payloadObject.serverUUID || payloadObject.serverUUID === Options.getServerUUID())
				) {
					Options.setOption(payloadObject.key, payloadObject.value);
				}
			} catch (error) {
				console.error('Could not parse payload for setOption', error);
			}
		}
	}
}

export default Admin;
