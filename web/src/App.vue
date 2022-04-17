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
