<script setup lang="ts">
// This starter template is using Vue 3 <script setup> SFCs
// Check out https://v3.vuejs.org/api/sfc-script-setup.html#sfc-script-setup
let socket = new WebSocket("ws://localhost:8081");
socket.onopen = function (e) {
  console.log("Connection Established");
}
socket.onerror = function () {
  console.error("ws error");
}
socket.onclose = function () {
  console.log("ws closed");
}
socket.onmessage = function (event) { console.log(event.data) }

let batch_socket = new WebSocket("ws://localhost:8081/batch/1");
batch_socket.onopen = function () {
  console.log("Connection Established");
}
batch_socket.onmessage = function (event) { console.log(event.data); }
batch_socket.onerror = function () {
  console.error("ws error");
}
batch_socket.onclose = function () {
  console.log("ws closed");
}

let individual_socket = new WebSocket("ws://localhost:8081/individual/12345610");
individual_socket.onopen = function () {
  console.log("Connection Established");
}
individual_socket.onmessage = function (event) { console.log(event.data); }
individual_socket.onerror = function () {
  console.error("ws error");
}
individual_socket.onclose = function () {
  console.log("ws closed");
}

</script>

<template>

</template>

<style>
#app {
  font-family: Avenir, Helvetica, Arial, sans-serif;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-align: center;
  color: #2c3e50;
  margin-top: 60px;
}
</style>
