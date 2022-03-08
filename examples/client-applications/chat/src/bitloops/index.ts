import Bitloops from 'bitloops';

import { DemoChat } from './proto/demoChat'; 
import bitloopsConfig from '../bitloops/bitloopsConfig';
const bitloopsApp = Bitloops.initialize(bitloopsConfig);
const demoChat = new DemoChat.DemoChatClient(bitloopsApp);
const bitloops = { demoChat, app: bitloopsApp, auth: bitloopsApp.auth, subscribe: bitloopsApp.subscribe.bind(bitloopsApp) };
export default bitloops;

export { DemoChat };