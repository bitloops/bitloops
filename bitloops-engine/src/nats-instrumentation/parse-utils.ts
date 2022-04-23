import { TextMapGetter, TextMapSetter } from '@opentelemetry/api';
import { MsgHdrs } from 'nats';

export default class ParseUtils {
	static getter: TextMapGetter<MsgHdrs> = {
		keys(carrier) {
			return carrier.keys();
		},
		get(h, key) {
			return h.get(key);
		},
	};

	static setter: TextMapSetter<MsgHdrs> = {
		set: (h, key, value) => {
			h.append(key, value);
		},
	};
}
