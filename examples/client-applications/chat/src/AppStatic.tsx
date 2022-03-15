import React, { useEffect, useRef, useState } from 'react';

import './components/chat.scss';
import Header from './components/Header';
import GoogleButton from './components/GoogleButton';
import Message from './components/Message';

let unsubscribe: any;

const getInitialMessagesValue = (): [] | {sendAt: number, senderUid: string, message: string, senderNickname: string}[] => {
  return [
    {sendAt: 1647136543325, message: 'This is a mock message', senderUid: '1', senderNickname: 'Me'},
    {sendAt: 1647136558332, message: 'This is a mock reply', senderUid: '2', senderNickname: 'Someone else'},
  ];
};

function App() {
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const [nickname, setNickname] = useState('');
  const [message, setMessage] = useState('');
  const [user, setUser] = useState<{firstName: string, uid: string} | null>({firstName: 'Me', uid: '1'});
  const [messages, setMessages] = useState(() => getInitialMessagesValue());
  
  const scrollToBottom = () => {
    const currentElement = messagesEndRef?.current;
    currentElement?.scrollIntoView({ behavior: "smooth" });
  }

  React.useEffect(() => {
  // TODO add onAuthStateChange listener to set user data
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => { 
    console.log('Received message', messages);
    scrollToBottom();
  }, [messages]);

  useEffect(() => {
    async function subscribe() {
      // TODO add subscription
      unsubscribe = () => { console.log('Unsubscribed') };
      // unsubscribe = await bitloops.subscribe(bitloops.demoChat.Events.ChatDemoNewPublicMessage(), (msg: DemoChat.Subscription_ChatDemoNewPublicMessage) => {
      //   setMessages(prevState => [...prevState, msg]);
      // });
    }
    if (user) {
      setNickname(user.firstName);
      subscribe();
    } else {
      if (unsubscribe) unsubscribe();
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [user]);

  const handleSendMessage = (payload: {message: string, nickname: string, senderUid: string}): void => {
    console.log('Sending', payload);
    // TODO send public message to backend
    setMessage('');
  }

  return (
    <>
      {user ? 
        <div className="container clearfix">
          {/* TODO handle logout */}
          <div><Header user={user} logout={() => console.log('clicked logout')}/></div>
          <div className="chat">
            <div className="chat-history">
              <ul style={{listStyle: 'none'}}>{messages && messages.map(message => (
              <Message 
                key={message.sendAt+message.senderUid}
                isMine={message.senderUid === user.uid}
                sendAt={message.sendAt}
                senderNickname={message.senderNickname}
                message={message.message}
              />
             ))}</ul>
             <div ref={messagesEndRef} />
             </div>
             
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
          {/* TODO handle login */}
          {!user && <GoogleButton loginWithGoogle={() => console.log('authenticating with Google')}/>}
        </div>
      }
    </>
  );
}

export default App;
