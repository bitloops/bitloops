import React, { useEffect, useRef, useState } from 'react';
import { BitloopsUser, Unsubscribe } from 'bitloops/dist/definitions';

import './components/chat.scss';
import bitloops, { DemoChat } from './bitloops';
import Header from './components/Header';
import GoogleButton from './components/GoogleButton';
import Message from './components/Message';

let unsubscribe: Unsubscribe;

function App() {
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [nickname, setNickname] = useState('');
  const [message, setMessage] = useState('');
  const [user, setUser] = useState<BitloopsUser | null>(null);
  const [messages, setMessages] = useState<DemoChat.Subscription_ChatDemoNewPublicMessage[] | []>([]);
  
  const scrollToBottom = () => {
    const currentElement = messagesEndRef?.current;
    currentElement?.scrollIntoView({ behavior: "smooth" });
  }
  /**
   * Upon initialization set onAuthStateChange in order
   * to keep track of auth state locally
   */
  React.useEffect(() => {
    bitloops.auth.onAuthStateChange((user: any) => {
      setUser(user);
    });
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    async function subscribe() {
      unsubscribe = await bitloops.subscribe(bitloops.demoChat.Events.ChatDemoNewPublicMessage(), (msg: DemoChat.Subscription_ChatDemoNewPublicMessage) => {
        setMessages(prevState => [...prevState, msg])});
    }
    if (user) {
      setNickname(user.firstName);
      subscribe();
    } else {
      if (unsubscribe) unsubscribe();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user]);

  useEffect(() => { 
    console.log('Received message', messages);
    scrollToBottom();
  }, [messages]);

  const handleSendMessage = (payload: {message: string, nickname: string, senderUid: string}): void => {
    console.log('Sending', payload);
    bitloops.demoChat.chatDemoPublicMessageSent({message, nickname, senderUid: user?.uid || ''});
    setMessage('');
  }

  return (
    <>
      {user ? 
        <div className="container clearfix">
          <div><Header user={user} logout={() => bitloops.app.auth.clearAuthentication()}/></div>
          <div className="chat">
            <div className="chat-history">
              <ul style={{listStyle: 'none'}}>{messages && messages.map(message => (
              <Message 
                key={message.senderUid+(Math.random()*100000)}
                isMine={message.senderUid === user.uid}
                sendAt={message.sendAt}
                senderNickname={message.senderNickname}
                message={message.message}
              />
             ))}</ul>
             <div ref={messagesEndRef} /></div>
            <div className="chat-message clearfix">
              <input 
                value={message}
                name="message-to-send"
                id="message-to-send"
                onChange={(event) => {setMessage(event.target.value)}}
                onBlur={(event) => { setMessage(event.target.value)}}
                onKeyPress={(event) => {
                  if (event.key === 'Enter') {
                    handleSendMessage({message, nickname, senderUid: user.uid});
                  } 
                }}
                placeholder="Type your message"
              />           
              <button
                onKeyPress={() => {
                  handleSendMessage({message, nickname, senderUid: user.uid});
                }}
                onClick={() => {
                  handleSendMessage({message, nickname, senderUid: user.uid});
              }}
              >Send</button>
            </div>
          </div>
        </div> :
        <div>
          {!user && <GoogleButton loginWithGoogle={() => bitloops.demoChat.bitloopsApp.auth.authenticateWithGoogle()}/>}
        </div>
      }
    </>
  );
}

export default App;
