import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';
export default defineConfig({plugins:[solid()],server:{port:5173,strictPort:true,proxy:{'/games':{target:'http://127.0.0.1:8080',ws:true},'/health':'http://127.0.0.1:8080'}}});
