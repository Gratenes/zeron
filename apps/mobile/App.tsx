import { StatusBar } from 'expo-status-bar';
import { useState } from 'react';
import { Pressable, SafeAreaView, ScrollView, StyleSheet, Text, TextInput, View } from 'react-native';

const tabs = ['Chat', 'Files', 'Changes', 'Settings'];

export default function App() {
  const [tab, setTab] = useState('Chat');
  const [draft, setDraft] = useState('');
  const [messages, setMessages] = useState([
    { role: 'You', body: 'Build a React Native client with full GPUI parity.' },
    { role: 'Kratos', body: 'The mobile shell is ready. Pair a node next to make this conversation live.' },
  ]);

  const send = () => {
    const body = draft.trim();
    if (!body) return;
    setMessages((current) => [...current, { role: 'You', body }]);
    setDraft('');
  };

  return (
    <SafeAreaView style={styles.screen}>
      <StatusBar style="light" />
      <View style={styles.header}>
        <Pressable accessibilityLabel="Open workspace menu" style={styles.iconButton}>
          <Text style={styles.icon}>☰</Text>
        </Pressable>
        <View style={styles.brand}>
          <Text style={styles.eyebrow}>WORKSPACE</Text>
          <Text style={styles.title}>Kratos</Text>
        </View>
        <View style={styles.offline}>
          <View style={styles.offlineDot} />
          <Text style={styles.offlineText}>Not paired</Text>
        </View>
      </View>

      <View style={styles.content}>
        <Text style={styles.workspace}>New task</Text>
        <Text style={styles.context}>Mobile preview · local state</Text>
        {tab === 'Chat' ? (
          <ScrollView contentContainerStyle={styles.messages} showsVerticalScrollIndicator={false}>
            {messages.map((message, index) => (
              <View key={`${message.role}-${index}`} style={message.role === 'You' ? styles.userMessage : styles.agentMessage}>
                <Text style={styles.messageRole}>{message.role}</Text>
                <Text style={styles.messageBody}>{message.body}</Text>
              </View>
            ))}
          </ScrollView>
        ) : (
          <View style={styles.emptyPanel}>
            <Text style={styles.emptyTitle}>{tab}</Text>
            <Text style={styles.emptyBody}>This GPUI surface will populate after node pairing is connected.</Text>
          </View>
        )}
      </View>

      {tab === 'Chat' && (
        <View style={styles.composer}>
          <TextInput
            accessibilityLabel="Message Kratos"
            multiline
            onChangeText={setDraft}
            placeholder="Message Kratos…"
            placeholderTextColor="#737373"
            style={styles.input}
            value={draft}
          />
          <Pressable accessibilityLabel="Send message" onPress={send} style={[styles.send, !draft.trim() && styles.sendDisabled]}>
            <Text style={styles.sendText}>↑</Text>
          </Pressable>
        </View>
      )}

      <View style={styles.tabs}>
        {tabs.map((item) => (
          <Pressable key={item} accessibilityRole="tab" accessibilityState={{ selected: tab === item }} onPress={() => setTab(item)} style={styles.tab}>
            <Text style={[styles.tabText, tab === item && styles.activeTab]}>{item}</Text>
          </Pressable>
        ))}
      </View>
    </SafeAreaView>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: '#111110' },
  header: { alignItems: 'center', borderBottomColor: '#282725', borderBottomWidth: 1, flexDirection: 'row', gap: 12, minHeight: 62, paddingHorizontal: 16 },
  iconButton: { alignItems: 'center', height: 36, justifyContent: 'center', width: 36 },
  icon: { color: '#d4d4d0', fontSize: 20 },
  brand: { flex: 1 },
  eyebrow: { color: '#787873', fontSize: 10, fontWeight: '700', letterSpacing: 1.1 },
  title: { color: '#f2f2ee', fontSize: 17, fontWeight: '600' },
  offline: { alignItems: 'center', backgroundColor: '#20201e', borderColor: '#353532', borderRadius: 99, borderWidth: 1, flexDirection: 'row', gap: 6, paddingHorizontal: 9, paddingVertical: 6 },
  offlineDot: { backgroundColor: '#d6a653', borderRadius: 4, height: 7, width: 7 },
  offlineText: { color: '#c5c3bd', fontSize: 12, fontWeight: '600' },
  content: { flex: 1, paddingHorizontal: 16, paddingTop: 22 },
  workspace: { color: '#f2f2ee', fontSize: 24, fontWeight: '600' },
  context: { color: '#85857f', fontSize: 13, marginTop: 4 },
  messages: { gap: 12, paddingBottom: 20, paddingTop: 22 },
  userMessage: { alignSelf: 'flex-end', backgroundColor: '#2b322f', borderColor: '#3d4842', borderRadius: 14, borderTopRightRadius: 3, borderWidth: 1, maxWidth: '90%', padding: 13 },
  agentMessage: { backgroundColor: '#191918', borderColor: '#30302d', borderRadius: 14, borderTopLeftRadius: 3, borderWidth: 1, maxWidth: '94%', padding: 13 },
  messageRole: { color: '#a1a19a', fontSize: 12, fontWeight: '700', marginBottom: 5 },
  messageBody: { color: '#e8e7e2', fontSize: 16, lineHeight: 23 },
  emptyPanel: { alignItems: 'center', borderColor: '#302f2c', borderRadius: 12, borderWidth: 1, marginTop: 28, padding: 24 },
  emptyTitle: { color: '#e8e7e2', fontSize: 18, fontWeight: '600' },
  emptyBody: { color: '#989791', fontSize: 14, lineHeight: 20, marginTop: 8, textAlign: 'center' },
  composer: { alignItems: 'flex-end', backgroundColor: '#1b1b19', borderColor: '#353532', borderRadius: 15, borderWidth: 1, flexDirection: 'row', gap: 10, marginHorizontal: 16, marginVertical: 12, padding: 10 },
  input: { color: '#f2f2ee', flex: 1, fontSize: 16, lineHeight: 22, maxHeight: 110, minHeight: 28, paddingHorizontal: 2, paddingVertical: 2 },
  send: { alignItems: 'center', backgroundColor: '#d9ded3', borderRadius: 10, height: 34, justifyContent: 'center', width: 34 },
  sendDisabled: { backgroundColor: '#41413d' },
  sendText: { color: '#171715', fontSize: 22, fontWeight: '700', lineHeight: 24 },
  tabs: { borderTopColor: '#282725', borderTopWidth: 1, flexDirection: 'row', paddingBottom: 9, paddingTop: 8 },
  tab: { alignItems: 'center', flex: 1, paddingVertical: 8 },
  tabText: { color: '#777770', fontSize: 12, fontWeight: '600' },
  activeTab: { color: '#e8e7e2' },
});
