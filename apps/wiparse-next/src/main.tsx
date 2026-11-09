import React from 'react';
import { createRoot } from 'react-dom/client';
import { App } from './App';
import './style.css';
import {readLegacy} from './bridge';
import {applyAppearance} from './appearance';
import {applyLanguage} from './language';
import {object,string} from './model';
// Retry appearance after host bootstrap; never block the application shell.
const appearanceTimer=setInterval(()=>{void readLegacy('ui.preferences.get_safe').then(value=>{applyAppearance(string(object(object(value).fields).theme));applyLanguage(string(object(object(value).fields).language));clearInterval(appearanceTimer);}).catch(()=>{});},2000);

createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
