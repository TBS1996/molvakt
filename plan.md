Language learning bot

This is an application for learning languages through whatsapp using an intermediary bot between you and a real life person.
molvakt from norwegian word målvakt, which typically means goalkeeper but can also mean language guard.


# prototype example flow

Alice is norwegian
bob is american
molvakt is a whatsapp bot
bob wants to learn norwegian.
they both have whatsapp
bob adds molvakt on whatsapp, is asked to supply a number to chat with and language
bob sends Alice's number, and selects norwegian as the language.
Alice receive's a message from molvakt, asking if she accepts starting a conversation with bob through langprox
Alice sends a norwegian message to bob through molvakt (can only send one at a time)
molvakt sends this message to bob. 
bob must clarify if he understood the message. he gets 3 predefined answers to pick from
1. he understands the message completely
2. he doesn't understand the message
3. he thinks he might understand the message

1: he understood the message:
he can start crafting an answer back in norwegian

2: he doesn't understand the message:
molvakt will translate the message and also teach the grammatical structure and the meaning of each word both in isolation and in the context of this message. 
after that, he can start crafting an answer back in norwegian

3: he thinks he might understand the message
bob will be asked to translate the message. 
if he succeeds, he can start crafting an answer back.
if he does not succeed, molvakt will give the translation, grammatical structure, and meaning of each word. and custom tips based on his wrong translation to help him.

When bob finished writing a norwegian message, he will try to send it. 
If the message is grammatically correct norwegian, it will be sent to Alice.
If the message contains errors, molvakt will give him hints on why it's not correct, and he can try again, until the message is correct and it will be sent to Alice.
Alice receive the message on molvakt, and also gets information on both bob's understanding of her message and his attempts at writing the message. 

``` example reply from bob proxied through molvakt
Reply from Bob: Hei, jeg har det bra, takk som spør. Hvordan har du det?

Bob guessed the meaning of your message and got it right on first try.
Bob needed 3 iterations to craft this message.
````
Alice reads this message, and can now start to reply with another message, and the conversation will keep going.


# Future plans

These are things that i would want in the future, but not for original prototype

* Sending multiple messages. First prototype will only be about making one message and taking turns to keep things simple.
* Multiple conversations with different people.
* Flashcard system. Program will track words used throughout conversation and while waiting for reply from someone you can review vocab used so far. 
* Voice messages
* Payment system, LLMs aren't free so if i scale this then people must pay
* Alternatively, support using token from some LLM if the user has already paid for one



