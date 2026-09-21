import { StatusBar } from 'expo-status-bar';
import { StyleSheet, Text, View } from 'react-native';

export default function App() {
  return (
    <View style={styles.container}>
      <Text style={styles.title}>Kratos</Text>
      <Text style={styles.subtitle}>GPUI parity foundation</Text>
      <StatusBar style="light" />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#171717',
    alignItems: 'center',
    justifyContent: 'center',
  },
  title: {
    color: '#f5f5f5',
    fontSize: 28,
    fontWeight: '600',
  },
  subtitle: {
    color: '#a3a3a3',
    fontSize: 16,
    marginTop: 8,
  },
});
