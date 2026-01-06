// services.js - Service definitions
angular.module('myApp.services', [])

.service('UserService', ['$http', '$q', function($http, $q) {
    this.getAll = function() {
        return $http.get('/api/users').then(function(response) {
            return response.data;
        });
    };

    this.getById = function(id) {
        return $http.get('/api/users/' + id).then(function(response) {
            return response.data;
        });
    };

    this.create = function(user) {
        return $http.post('/api/users', user);
    };
}])

.factory('AuthService', ['$http', '$q', function($http, $q) {
    var isAuthenticated = false;

    return {
        login: function(credentials) {
            return $http.post('/api/login', credentials).then(function(response) {
                isAuthenticated = true;
                return response.data;
            });
        },
        logout: function() {
            return $http.post('/api/logout').then(function() {
                isAuthenticated = false;
            });
        },
        isLoggedIn: function() {
            return isAuthenticated;
        }
    };
}]);
